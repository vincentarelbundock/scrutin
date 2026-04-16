"""scrutin great_expectations runner.

Persistent subprocess that reads test file paths from stdin (one per line)
and executes each as a plain Python script. After execution it walks the
script's module globals for great_expectations result objects
(`CheckpointResult`, `ExpectationSuiteValidationResult`) and emits one
NDJSON event per `ExpectationValidationResult`.

Wire protocol: see docs/reporting-spec.md and the Rust mirror at
crates/scrutin-core/src/engine/protocol.rs. Three message types:
  - {"type":"event", "outcome": pass|fail|error|xfail, ...}
  - {"type":"summary", "duration_ms":..., "counts": {...}}
  - {"type":"done"}

This is the Python analogue of the pointblank runner in
crates/scrutin-core/src/r/runner.R: declarative result objects produced by
the user's file, runner introspects them. We deliberately do not construct
checkpoints ourselves — composing the validation is the user's job.
"""

import os
import sys

# sys.path hygiene: same defence as scrutin_pytest.py. The runner's
# directory is prepended to sys.path[0] by Python; scrub it so any stale
# file in the cache dir (or a user-placed `great_expectations.py`) can
# never shadow the real package import.
_here = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p and os.path.abspath(p) != _here]

import json
import runpy
import time
import traceback


def emit(obj):
    sys.stdout.write(json.dumps(obj, default=str))
    sys.stdout.write("\n")
    sys.stdout.flush()


def emit_event(file, outcome, name, *, parent=None, message=None,
               line=None, duration_ms=0, metrics=None, failures=None):
    subject = {"kind": "expectation", "name": name}
    if parent is not None:
        subject["parent"] = parent
    obj = {
        "type": "event",
        "file": file,
        "outcome": outcome,
        "subject": subject,
        "duration_ms": int(duration_ms),
    }
    if message is not None:
        obj["message"] = message
    if line is not None:
        obj["line"] = line
    if metrics is not None:
        obj["metrics"] = metrics
    if failures:
        obj["failures"] = failures
    emit(obj)


def emit_summary(file, counts, duration_ms):
    emit({
        "type": "summary",
        "file": file,
        "duration_ms": int(duration_ms),
        "counts": {
            "pass":  int(counts.get("pass",  0)),
            "fail":  int(counts.get("fail",  0)),
            "error": int(counts.get("error", 0)),
            "skip":  int(counts.get("skip",  0)),
            "xfail": int(counts.get("xfail", 0)),
            "warn":  int(counts.get("warn",  0)),
        },
    })


# ── Result-object introspection ─────────────────────────────────────────────

def _looks_like_suite_result(obj):
    """Duck-typed check for ExpectationSuiteValidationResult.

    Both GE v0.18 and v1.x expose a `.results` list of per-expectation
    results plus a `.success` bool on this object. Avoiding isinstance
    checks lets the runner work across versions without importing GE
    types eagerly.
    """
    return hasattr(obj, "results") and hasattr(obj, "success") \
        and isinstance(getattr(obj, "results"), (list, tuple))


def _looks_like_checkpoint_result(obj):
    """Duck-typed check for CheckpointResult.

    `.run_results` is a dict of identifier → per-suite result; each value
    has a `validation_result` (or is itself a dict containing one).
    """
    return hasattr(obj, "run_results") \
        and isinstance(getattr(obj, "run_results"), dict)


def _extract_suite_results(checkpoint_result):
    """Yield ExpectationSuiteValidationResult objects from a CheckpointResult."""
    for entry in checkpoint_result.run_results.values():
        # v1.x: entry is itself the suite result.
        if _looks_like_suite_result(entry):
            yield entry
            continue
        # v0.18: entry is a dict with "validation_result".
        if isinstance(entry, dict):
            inner = entry.get("validation_result")
            if inner is not None and _looks_like_suite_result(inner):
                yield inner
                continue
        # Fallback: object with .validation_result attr.
        inner = getattr(entry, "validation_result", None)
        if inner is not None and _looks_like_suite_result(inner):
            yield inner


def _expectation_name(exp_result):
    """Build a stable subject name from an ExpectationValidationResult.

    Format: `expectation_type(column)` or `expectation_type` when no column
    kwarg is present. Mirrors pointblank's `step_type(cols)` shape.
    """
    cfg = getattr(exp_result, "expectation_config", None)
    etype = None
    column = None
    if cfg is not None:
        etype = getattr(cfg, "expectation_type", None) or getattr(cfg, "type", None)
        kwargs = getattr(cfg, "kwargs", None) or {}
        if isinstance(kwargs, dict):
            column = kwargs.get("column")
            if column is None:
                # Multi-column expectations expose `column_list` / `column_A`/`column_B`.
                cols = kwargs.get("column_list")
                if cols:
                    column = ",".join(str(c) for c in cols)
                elif kwargs.get("column_A") and kwargs.get("column_B"):
                    column = "{},{}".format(kwargs["column_A"], kwargs["column_B"])
    etype = etype or "<expectation>"
    if column:
        return "{}({})".format(etype, column)
    return etype


def _expectation_metrics(exp_result):
    """Pull element/unexpected counts off an ExpectationValidationResult."""
    result = getattr(exp_result, "result", None)
    if not isinstance(result, dict):
        return None
    metrics = {}
    if "element_count" in result:
        try:
            metrics["total"] = float(result["element_count"])
        except (TypeError, ValueError):
            pass
    if "unexpected_count" in result:
        try:
            metrics["failed"] = float(result["unexpected_count"])
        except (TypeError, ValueError):
            pass
    if "unexpected_percent" in result:
        try:
            # GE reports a percent (0..100); scrutin's `fraction` is 0..1.
            metrics["fraction"] = float(result["unexpected_percent"]) / 100.0
        except (TypeError, ValueError):
            pass
    return metrics or None


def _emit_suite(file, suite_result, parent, counts):
    """Emit one event per ExpectationValidationResult in a suite."""
    for exp in suite_result.results:
        name = _expectation_name(exp)
        exc_info = getattr(exp, "exception_info", None) or {}
        raised = bool(exc_info.get("raised_exception")) if isinstance(exc_info, dict) else False

        # Check meta.expected_to_fail for xfail support.
        cfg = getattr(exp, "expectation_config", None)
        meta = (getattr(cfg, "meta", None) or {}) if cfg is not None else {}
        expected_to_fail = bool(meta.get("expected_to_fail"))

        if raised:
            outcome = "error"
            msg = ""
            if isinstance(exc_info, dict):
                msg = "{}\n{}".format(
                    exc_info.get("exception_message", ""),
                    exc_info.get("exception_traceback", ""),
                ).strip()
        elif bool(getattr(exp, "success", False)):
            outcome = "pass"
            msg = None
        elif expected_to_fail:
            outcome = "xfail"
            msg = None
        else:
            outcome = "fail"
            # Compose a short failure message from the result dict.
            result = getattr(exp, "result", None)
            if isinstance(result, dict):
                bits = []
                if "unexpected_count" in result:
                    bits.append("unexpected_count={}".format(result["unexpected_count"]))
                if "unexpected_percent" in result:
                    bits.append("unexpected_percent={:.2f}".format(
                        float(result.get("unexpected_percent") or 0.0)))
                partial = result.get("partial_unexpected_list")
                if partial:
                    bits.append("examples={}".format(partial[:5]))
                msg = "; ".join(bits) or "expectation failed"
            else:
                msg = "expectation failed"

        counts[outcome] = counts.get(outcome, 0) + 1
        emit_event(file, outcome, name, parent=parent, message=msg,
                   metrics=_expectation_metrics(exp))


# ── File execution ──────────────────────────────────────────────────────────

def run_test(path):
    file_name = os.path.basename(path)
    t0 = time.time()
    counts = {"pass": 0, "fail": 0, "error": 0, "skip": 0, "xfail": 0, "warn": 0}

    try:
        ns = runpy.run_path(path, run_name="__scrutin_ge__")
    except BaseException as e:
        elapsed = int((time.time() - t0) * 1000)
        counts["error"] += 1
        tb = traceback.format_exc()
        emit_event(file_name, "error", "<file>",
                   message="{}: {}\n{}".format(type(e).__name__, e, tb),
                   duration_ms=elapsed)
        emit_summary(file_name, counts, elapsed)
        emit({"type": "done"})
        return

    # Walk module globals for result objects. Order: stable by name so the
    # event stream is reproducible across runs.
    found_any = False
    for name in sorted(k for k in ns.keys() if not k.startswith("_")):
        obj = ns[name]
        if _looks_like_checkpoint_result(obj):
            found_any = True
            try:
                for suite in _extract_suite_results(obj):
                    _emit_suite(file_name, suite, parent=name, counts=counts)
            except BaseException as e:
                counts["error"] += 1
                emit_event(file_name, "error", name,
                           message="failed to read CheckpointResult: {}: {}".format(
                               type(e).__name__, e))
        elif _looks_like_suite_result(obj):
            found_any = True
            try:
                _emit_suite(file_name, obj, parent=name, counts=counts)
            except BaseException as e:
                counts["error"] += 1
                emit_event(file_name, "error", name,
                           message="failed to read suite result: {}: {}".format(
                               type(e).__name__, e))

    if not found_any:
        counts["error"] += 1
        emit_event(file_name, "error", "<no_result>",
                   message="no great_expectations result objects found in module globals "
                           "(expected a CheckpointResult or ExpectationSuiteValidationResult)")

    elapsed = int((time.time() - t0) * 1000)
    emit_summary(file_name, counts, elapsed)
    emit({"type": "done"})


# ── Worker loop ─────────────────────────────────────────────────────────────

def main():
    pkg_dir = os.environ.get("SCRUTIN_PKG_DIR", ".")
    if pkg_dir not in sys.path:
        sys.path.insert(0, pkg_dir)
    src_dir = os.path.join(pkg_dir, "src")
    if os.path.isdir(src_dir) and src_dir not in sys.path:
        sys.path.insert(0, src_dir)

    # Worker startup hook: matches the pytest runner's contract.
    startup = os.environ.get("SCRUTIN_WORKER_STARTUP")
    if startup:
        try:
            runpy.run_path(startup, run_name="__scrutin_worker_startup__")
        except BaseException as e:
            emit_event("<worker_startup>", "error", "<worker_startup>",
                       message="{}: {}".format(type(e).__name__, e))
            sys.exit(2)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line == "!shutdown":
            teardown = os.environ.get("SCRUTIN_WORKER_TEARDOWN")
            if teardown:
                try:
                    runpy.run_path(teardown, run_name="__scrutin_worker_teardown__")
                except BaseException as e:
                    sys.stderr.write(
                        "[scrutin] worker_teardown failed: {}: {}\n".format(
                            type(e).__name__, e
                        )
                    )
            sys.exit(0)
        run_test(line)


if __name__ == "__main__":
    main()
