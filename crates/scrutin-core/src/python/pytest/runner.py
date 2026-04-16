"""scrutin pytest runner.

Persistent subprocess that reads test file paths from stdin (one per line)
and runs pytest on each file, emitting NDJSON to stdout.

Wire protocol: see docs/reporting-spec.md and the Rust mirror at
crates/scrutin-core/src/engine/protocol.rs. Three message types:
  - {"type":"event", "outcome": pass|fail|error|skip|xfail|warn, ...}
  - {"type":"summary", "duration_ms":..., "counts": {...}}
  - {"type":"done"}
"""

import json
import os
import runpy
import sys
import time
import traceback


# Snapshot the original stdout so user test code that reassigns sys.stdout
# (or a leaked contextlib.redirect_stdout) can't swallow scrutin's NDJSON.
# sys.__stdout__ is Python's canonical handle to the interpreter's original
# stdout and is unaffected by `sys.stdout = x`. This mirrors the fd-1 bypass
# in runner_r.R for R's sink() stack.
_real_stdout = sys.__stdout__ or sys.stdout


def emit(obj):
    # default=str handles pytest's Path / LocalPath objects in longrepr.
    _real_stdout.write(json.dumps(obj, default=str))
    _real_stdout.write("\n")
    _real_stdout.flush()


def emit_event(file, outcome, name, *, parent=None, message=None,
               line=None, duration_ms=0, metrics=None, failures=None):
    subject = {"kind": "function", "name": name}
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


class ScrutinPlugin:
    def __init__(self, file_name):
        self.file_name = file_name
        self.counts = {"pass": 0, "fail": 0, "error": 0,
                       "skip": 0, "xfail": 0, "warn": 0}
        # Track which nodeids we've already emitted so setup-errors don't
        # double-count against a later "call" phase.
        self._emitted = set()

    def pytest_runtest_logreport(self, report):
        nodeid = report.nodeid
        test_name = nodeid.partition("::")[2] or nodeid

        # pytest emits setup/call/teardown phases. Emit on:
        #   - call phase (the actual test result)
        #   - setup phase when it failed or was skipped (test never ran)
        if report.when == "call":
            pass
        elif report.when == "setup" and (report.failed or report.skipped):
            pass
        else:
            return

        if nodeid in self._emitted:
            return
        self._emitted.add(nodeid)

        ms = int((report.duration or 0.0) * 1000)
        wasxfail = bool(getattr(report, "wasxfail", False))

        if report.passed:
            # Strict xfail that unexpectedly passed shows up as
            # report.passed=True with `wasxfail` set; pytest also raises
            # XPASS reports through outcome attr in newer versions. We
            # treat unexpected pass as a regular pass to keep CI green.
            self.counts["pass"] += 1
            emit_event(self.file_name, "pass", test_name, duration_ms=ms)
            return

        if report.skipped:
            # `wasxfail` distinguishes "this is xfail and it failed as
            # expected" from a plain skip. The former is the user-visible
            # success case for xfail tests.
            outcome = "xfail" if wasxfail else "skip"
            self.counts[outcome] += 1
            msg = ""
            lr = report.longrepr
            if isinstance(lr, tuple) and len(lr) >= 3:
                msg = str(lr[2])
            elif lr is not None:
                msg = str(lr)
            emit_event(self.file_name, outcome, test_name,
                       message=msg, duration_ms=ms)
            return

        # Failed
        line = None
        msg = ""
        lr = report.longrepr
        try:
            crash = getattr(lr, "reprcrash", None)
            if crash is not None:
                line = getattr(crash, "lineno", None)
        except Exception:
            pass
        try:
            msg = str(lr) if lr is not None else ""
        except Exception:
            msg = "<unprintable failure repr>"
        # report.when == "setup" failures are evaluator errors, not
        # assertion failures, so map them to `error`.
        outcome = "error" if report.when == "setup" else "fail"
        self.counts[outcome] += 1
        emit_event(self.file_name, outcome, test_name,
                   message=msg, line=line, duration_ms=ms)


def run_test(path):
    file_name = os.path.basename(path)
    t0 = time.time()
    try:
        import pytest  # imported lazily so startup errors surface per-file
    except Exception as e:
        emit_event(file_name, "error", "<import>",
                   message="Failed to import pytest: {}".format(e),
                   duration_ms=int((time.time() - t0) * 1000))
        emit_summary(file_name, {"error": 1}, int((time.time() - t0) * 1000))
        return

    plugin = ScrutinPlugin(file_name)
    try:
        # -p no:terminalreporter suppresses pytest's own stdout output
        # (progress dots, summary line) so only our NDJSON reaches stdout.
        argv = [
            path,
            "--tb=short",
            "-p", "no:cacheprovider",
            "-p", "no:terminalreporter",
        ]
        # User escape hatch: extra_args from .scrutin/config.toml [pytest], JSON-encoded.
        extra = os.environ.get("SCRUTIN_PYTEST_EXTRA_ARGS")
        if extra:
            try:
                parsed = json.loads(extra)
                if isinstance(parsed, list):
                    argv.extend(str(a) for a in parsed)
            except Exception:
                pass
        rc = pytest.main(argv, plugins=[plugin])
        # pytest exit codes: 0 ok, 1 tests failed, 2 interrupted, 3 internal
        # error, 4 usage error, 5 no tests collected. 0/1/5 are normal — the
        # plugin already accounted for any failures. Anything else (notably 3)
        # means pytest itself broke without surfacing a per-test failure.
        if isinstance(rc, int) and rc not in (0, 1, 5):
            plugin.counts["error"] += 1
            emit_event(file_name, "error", "<pytest>",
                       message="pytest exited with code {}".format(rc),
                       duration_ms=int((time.time() - t0) * 1000))
    except SystemExit as e:
        code = e.code if isinstance(e.code, int) else 0
        if code not in (0, 1, 5):
            plugin.counts["error"] += 1
            emit_event(file_name, "error", "<pytest>",
                       message="pytest exited (SystemExit) with code {}".format(code),
                       duration_ms=int((time.time() - t0) * 1000))
    except BaseException as e:
        plugin.counts["error"] += 1
        emit_event(file_name, "error", "<pytest>",
                   message="pytest crashed: {}\n{}".format(e, traceback.format_exc()),
                   duration_ms=int((time.time() - t0) * 1000))

    elapsed = int((time.time() - t0) * 1000)
    emit_summary(file_name, plugin.counts, elapsed)


def _read_project_name(pkg_dir):
    """Best-effort extraction of the project package name from pyproject.toml.

    Uses the stdlib `tomllib` (3.11+), then `tomli` (3.10 and older), then
    falls back to a tiny line-based parser for environments with neither.
    """
    path = os.path.join(pkg_dir, "pyproject.toml")
    try:
        try:
            import tomllib  # 3.11+
        except ImportError:
            try:
                import tomli as tomllib  # type: ignore
            except ImportError:
                tomllib = None
        if tomllib is not None:
            with open(path, "rb") as f:
                data = tomllib.load(f)
            project = data.get("project") or {}
            name = project.get("name")
            if isinstance(name, str):
                return name
            poetry = (data.get("tool") or {}).get("poetry") or {}
            name = poetry.get("name")
            if isinstance(name, str):
                return name
            return None
        with open(path, "r", encoding="utf-8") as f:
            in_project = False
            for raw in f:
                line = raw.strip()
                if line.startswith("["):
                    in_project = line in ("[project]", "[tool.poetry]")
                    continue
                if in_project and (line.startswith("name=") or line.startswith("name ")):
                    rest = line.split("=", 1)[-1].strip()
                    return rest.strip("'\"")
    except Exception:
        pass
    return None


def _warm_up(pkg_dir):
    """Pre-import the project package and pytest so the first test file
    doesn't pay the whole import cost. Failures emit a single warn event."""
    try:
        import pytest  # noqa: F401
    except Exception:
        return
    name = _read_project_name(pkg_dir)
    if not name:
        return
    module_name = name.replace("-", "_")
    try:
        __import__(module_name)
    except Exception as e:
        # Package may not be importable (e.g., not installed in venv).
        # Surface the root cause once via a warn event tagged with a
        # synthetic file name.
        emit_event("<worker_warmup>", "warn", "<import>",
                   message="warm-up import of {!r} failed: {}: {}".format(
                       module_name, type(e).__name__, e))


def _run_test_tcp(path, port):
    """Fork a child that connects to Rust via TCP, runs the test, exits."""
    import socket as _socket

    pid = os.fork()
    if pid == 0:
        # Child: connect to Rust, redirect emit, run test, exit.
        sock = None
        f = None
        try:
            sock = _socket.create_connection(("127.0.0.1", port))
            f = sock.makefile("w")

            # Redirect emit to write to the TCP socket.
            def tcp_emit(obj):
                f.write(json.dumps(obj, default=str))
                f.write("\n")
                f.flush()

            global emit
            emit = tcp_emit

            run_test(path)
        except BaseException:
            pass
        finally:
            # Always close the socket so Rust sees EOF. on.exit/try-finally
            # are skipped by os._exit, so do it explicitly here.
            try:
                if f is not None:
                    f.close()
            except Exception:
                pass
            try:
                if sock is not None:
                    sock.close()
            except Exception:
                pass
            os._exit(0)
    else:
        # Parent: reap finished children without blocking.
        try:
            os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            pass


def main():
    pkg_dir = os.environ.get("SCRUTIN_PKG_DIR", ".")
    # Ensure the project root is on sys.path so `import mypkg` works.
    if pkg_dir not in sys.path:
        sys.path.insert(0, pkg_dir)
    # Also add src/ layout if present.
    src_dir = os.path.join(pkg_dir, "src")
    if os.path.isdir(src_dir) and src_dir not in sys.path:
        sys.path.insert(0, src_dir)

    _warm_up(pkg_dir)

    # Worker startup hook: run once before entering the read loop. Failure
    # emits an event tagged file="<worker_startup>" and exits 2, which the
    # Rust pool treats as a permanent poison sentinel.
    startup = os.environ.get("SCRUTIN_WORKER_STARTUP")
    if startup:
        try:
            runpy.run_path(startup, run_name="__scrutin_worker_startup__")
        except BaseException as e:
            emit_event("<worker_startup>", "error", "<worker_startup>",
                       message="{}: {}".format(type(e).__name__, e))
            sys.exit(2)

    tcp_port = os.environ.get("SCRUTIN_TCP_PORT", "")

    def do_shutdown():
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

    if tcp_port:
        # TCP fork mode: fork per file, child connects to Rust via TCP.
        port = int(tcp_port)
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            if line == "!shutdown":
                # Reap all children before shutdown.
                while True:
                    try:
                        os.waitpid(-1, 0)
                    except ChildProcessError:
                        break
                do_shutdown()
            _run_test_tcp(line, port)
    else:
        # Direct mode (Windows fallback): run in-process, emit to stdout.
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            if line == "!shutdown":
                do_shutdown()
            run_test(line)
            emit({"type": "done"})


if __name__ == "__main__":
    main()
