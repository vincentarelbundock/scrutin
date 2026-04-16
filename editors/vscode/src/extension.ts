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

  scrutinProcess = new ScrutinProcess();

  scrutinProcess.onExit((code) => {
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
    const baseUrl = await scrutinProcess.start(
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

function stopScrutin(): void {
  stopSse();
  testCtrl?.dispose();
  testCtrl = null;
  bus?.dispose();
  bus = null;
  scrutinProcess?.stop();
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
