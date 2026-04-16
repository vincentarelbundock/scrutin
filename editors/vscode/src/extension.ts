import * as vscode from "vscode";
import * as http from "http";
import { ScrutinProcess } from "./scrutin-process";
import { createPanel } from "./webview-panel";
import { EventBus, fetchSnapshot } from "./event-bus";
import { ScrutinTestController } from "./testing";
import type { WireEventKind, WireRunSummary } from "./types";

let scrutinProcess: ScrutinProcess | null = null;
let panel: vscode.WebviewPanel | null = null;
let statusBar: vscode.StatusBarItem | null = null;
let sseReq: ReturnType<typeof http.get> | null = null;
let bus: EventBus | null = null;
let testCtrl: ScrutinTestController | null = null;

export function activate(context: vscode.ExtensionContext): void {
  statusBar = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Left,
    50,
  );
  statusBar.command = "scrutin.showPanel";
  statusBar.text = "$(beaker) scrutin";
  context.subscriptions.push(statusBar);

  const register = (cmd: string, fn: () => Promise<void> | void) => {
    context.subscriptions.push(
      vscode.commands.registerCommand(cmd, fn),
    );
  };

  register("scrutin.start", () => startScrutin(context));
  register("scrutin.stop", stopScrutin);
  register("scrutin.showPanel", () => showPanel(context));
  register("scrutin.restart", () => restartScrutin(context));
  register("scrutin.init", () => initScrutin(context));

  // Auto-start if configured.
  const cfg = vscode.workspace.getConfiguration("scrutin");
  if (cfg.get<boolean>("autoStart")) {
    startScrutin(context);
  }
}

export function deactivate(): void {
  stopSse();
  testCtrl?.dispose();
  testCtrl = null;
  bus?.dispose();
  bus = null;
  scrutinProcess?.dispose();
  scrutinProcess = null;
}

async function startScrutin(context: vscode.ExtensionContext): Promise<void> {
  if (scrutinProcess?.running) {
    vscode.window.showInformationMessage("Scrutin is already running.");
    showPanel(context);
    return;
  }

  const binaryPath = await findBinary(context);
  if (!binaryPath) return;

  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    vscode.window.showErrorMessage("No workspace folder open.");
    return;
  }

  const proc = new ScrutinProcess();
  scrutinProcess = proc;

  proc.onExit((code) => {
    // Ignore stale exits from processes that have since been replaced
    // (by Stop + Start, or by Restart). The current scrutinProcess may
    // point at a different instance; only this one's exit affects UI.
    if (scrutinProcess !== proc) return;
    stopSse();
    if (statusBar) {
      statusBar.text = "$(beaker) scrutin (stopped)";
    }
    if (code !== null && code !== 0) {
      vscode.window
        .showErrorMessage(
          `Scrutin exited with code ${code}.`,
          "Restart",
        )
        .then((choice) => {
          if (choice === "Restart") startScrutin(context);
        });
    }
  });

  try {
    const baseUrl = await proc.start(
      binaryPath,
      folder.uri.fsPath,
    );
    if (statusBar) {
      statusBar.text = "$(beaker) scrutin";
      statusBar.show();
    }

    bus = new EventBus();
    context.subscriptions.push(bus);
    try {
      const snapshot = await fetchSnapshot(baseUrl);
      bus.setSnapshot(snapshot);
      testCtrl = new ScrutinTestController(
        bus,
        folder.uri.fsPath,
        baseUrl,
      );
      testCtrl.buildTree(snapshot);
      context.subscriptions.push(testCtrl);
    } catch {
      // Snapshot fetch failed; test explorer unavailable until first run.
    }

    connectSse(baseUrl);
    showPanel(context);
  } catch (e: unknown) {
    const errMsg = e instanceof Error ? e.message : String(e);
    vscode.window.showErrorMessage(`Failed to start scrutin: ${errMsg}`);
  }
}

async function restartScrutin(
  context: vscode.ExtensionContext,
): Promise<void> {
  stopScrutin();
  // Small delay to let the previous process release its port cleanly.
  await new Promise((r) => setTimeout(r, 250));
  await startScrutin(context);
}

/// Runs `scrutin init <workspace>` to scaffold `.scrutin/config.toml`.
/// Pipes output to an output channel and offers to start scrutin after.
async function initScrutin(
  context: vscode.ExtensionContext,
): Promise<void> {
  const binaryPath = await findBinary(context);
  if (!binaryPath) return;

  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    vscode.window.showErrorMessage("No workspace folder open.");
    return;
  }

  const output = vscode.window.createOutputChannel("Scrutin Init");
  output.show(true);
  output.appendLine(`$ scrutin init ${folder.uri.fsPath}`);

  const { execFile } = require("child_process");
  await new Promise<void>((resolve) => {
    execFile(
      binaryPath,
      ["init", folder.uri.fsPath],
      { encoding: "utf-8" },
      (
        err: Error & { code?: number } | null,
        stdout: string,
        stderr: string,
      ) => {
        if (stdout) output.append(stdout);
        if (stderr) output.append(stderr);
        if (err) {
          output.appendLine(`\nscrutin init failed: ${err.message}`);
          vscode.window.showErrorMessage(
            `scrutin init failed. See 'Scrutin Init' output for details.`,
          );
        } else {
          output.appendLine("\nDone.");
          vscode.window
            .showInformationMessage(
              "Scrutin initialized.",
              "Start",
            )
            .then((choice) => {
              if (choice === "Start") startScrutin(context);
            });
        }
        resolve();
      },
    );
  });
}

function stopScrutin(): void {
  stopSse();
  testCtrl?.dispose();
  testCtrl = null;
  bus?.dispose();
  bus = null;
  // dispose() disposes the EventEmitter, so any stale `exit` event
  // that fires after the SIGTERM lands is a no-op. Otherwise the old
  // process's exit handler could run after a restart and tear down
  // the newly-started process's SSE / status bar.
  scrutinProcess?.dispose();
  scrutinProcess = null;
  // Dispose the panel too: its webview has the old baseUrl baked in,
  // so it would keep trying to reconnect to the dead port after
  // restart. A fresh panel on next start gets the new URL.
  panel?.dispose();
  panel = null;
  if (statusBar) {
    statusBar.text = "$(beaker) scrutin (stopped)";
  }
  vscode.window.showInformationMessage("Scrutin stopped.");
}

function showPanel(context: vscode.ExtensionContext): void {
  if (panel) {
    panel.reveal();
    return;
  }
  if (!scrutinProcess?.baseUrl) {
    vscode.window.showInformationMessage(
      "Scrutin is not running. Use 'Scrutin: Start' first.",
    );
    return;
  }
  panel = createPanel(context, scrutinProcess.baseUrl);
  panel.onDidDispose(() => {
    panel = null;
  });
}

async function findBinary(
  context: vscode.ExtensionContext,
): Promise<string | null> {
  const cfg = vscode.workspace.getConfiguration("scrutin");

  // 1. Explicit user override wins.
  const explicit = cfg.get<string>("binaryPath", "").trim();
  if (explicit) return explicit;

  // 2. Bundled binary (per-platform VSIX).
  const exe = process.platform === "win32" ? "scrutin.exe" : "scrutin";
  const bundled = vscode.Uri.joinPath(context.extensionUri, "bin", exe).fsPath;
  try {
    await vscode.workspace.fs.stat(vscode.Uri.file(bundled));
    if (process.platform !== "win32") {
      const fs = require("fs");
      fs.chmodSync(bundled, 0o755);
    }
    return bundled;
  } catch {
    // Not bundled (universal VSIX).
  }

  // 3. PATH.
  const { execSync } = require("child_process");
  try {
    const cmd =
      process.platform === "win32" ? "where scrutin" : "which scrutin";
    const out = execSync(cmd, { encoding: "utf-8" })
      .split(/\r?\n/)[0]
      .trim();
    if (out) return out;
  } catch {
    // Not on PATH.
  }

  vscode.window.showErrorMessage(
    "Could not find the `scrutin` binary. " +
      "Install it or set `scrutin.binaryPath` in settings.",
  );
  return null;
}

// ── Status bar SSE ──────────────────────────────────────────────────────────

function connectSse(baseUrl: string): void {
  stopSse();
  const url = `${baseUrl}/api/events`;
  sseReq = http.get(url, (res) => {
    let buf = "";
    res.on("data", (chunk: Buffer) => {
      buf += chunk.toString();
      // Parse SSE frames: each event ends with \n\n.
      const frames = buf.split("\n\n");
      buf = frames.pop() ?? "";
      for (const frame of frames) {
        handleSseFrame(frame);
      }
    });
    res.on("end", () => {
      // Reconnect after 2 seconds.
      setTimeout(() => {
        if (scrutinProcess?.running) connectSse(baseUrl);
      }, 2000);
    });
  });
  sseReq.on("error", () => {
    setTimeout(() => {
      if (scrutinProcess?.running) connectSse(baseUrl);
    }, 2000);
  });
}

function stopSse(): void {
  if (sseReq) {
    sseReq.destroy();
    sseReq = null;
  }
}

function handleSseFrame(frame: string): void {
  if (!statusBar) return;
  let eventType = "";
  let data = "";
  for (const line of frame.split("\n")) {
    if (line.startsWith("event:")) eventType = line.slice(6).trim();
    else if (line.startsWith("data:")) data = line.slice(5).trim();
  }
  if (!eventType || !data) return;

  try {
    const parsed = JSON.parse(data);

    // On run_started, rebuild the test tree *before* firing the bus
    // event so that onRunStarted enqueues items from the fresh tree,
    // not stale nodes that buildTree is about to replace.
    if (eventType === "run_started" && scrutinProcess?.baseUrl) {
      statusBar.text = "$(sync~spin) scrutin: running\u2026";
      fetchSnapshot(scrutinProcess.baseUrl).then((snap) => {
        bus?.setSnapshot(snap);
        testCtrl?.buildTree(snap);
        bus?.fire({ kind: eventType as WireEventKind, data: parsed });
      }).catch(() => {
        bus?.fire({ kind: eventType as WireEventKind, data: parsed });
      });
      return;
    }

    // Forward to the event bus so the test controller (and future
    // consumers) can react without their own SSE connection.
    bus?.fire({ kind: eventType as WireEventKind, data: parsed });

    switch (eventType) {
      case "run_complete": {
        const t = parsed.totals as WireRunSummary["totals"];
        const bad = (t?.fail ?? 0) + (t?.error ?? 0);
        if (bad > 0) {
          statusBar.text = `$(error) scrutin: ${t.pass} pass, ${bad} fail`;
        } else {
          statusBar.text = `$(check) scrutin: ${t.pass} pass`;
        }
        break;
      }
      case "run_cancelled":
        statusBar.text = "$(beaker) scrutin: cancelled";
        break;
    }
  } catch {
    // ignore malformed events
  }
}
