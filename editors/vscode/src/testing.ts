import * as vscode from "vscode";
import * as http from "http";
import * as path from "path";
import type { EventBus, SseEvent } from "./event-bus";
import type { WireFile, WireMessage, WireSnapshot } from "./types";

export class ScrutinTestController {
  private ctrl: vscode.TestController;
  private bus: EventBus;
  private projectRoot: string;
  private baseUrl: string;
  private currentRun: vscode.TestRun | null = null;
  private fileItems: Map<string, vscode.TestItem> = new Map();
  private suiteItems: Map<string, vscode.TestItem> = new Map();
  private disposables: vscode.Disposable[] = [];

  constructor(bus: EventBus, projectRoot: string, baseUrl: string) {
    this.bus = bus;
    this.projectRoot = projectRoot;
    this.baseUrl = baseUrl;

    this.ctrl = vscode.tests.createTestController("scrutin", "Scrutin");

    this.ctrl.createRunProfile(
      "Run",
      vscode.TestRunProfileKind.Run,
      (request, token) => this.handleRunRequest(request, token),
      true,
    );

    this.disposables.push(bus.onEvent((e) => this.onEvent(e)));
  }

  // ── Tree construction ───────────────────────────────────────────────

  buildTree(snapshot: WireSnapshot): void {
    this.ctrl.items.replace([]);
    this.fileItems.clear();
    this.suiteItems.clear();

    for (const suite of snapshot.pkg.suites) {
      const item = this.ctrl.createTestItem(
        `suite:${suite.name}`,
        suite.name,
      );
      this.suiteItems.set(suite.name, item);
      this.ctrl.items.add(item);
    }

    for (const file of snapshot.files) {
      this.addFileItem(file);
    }
  }

  private addFileItem(file: WireFile): void {
    const suiteItem = this.suiteItems.get(file.suite);
    if (!suiteItem) return;

    const absPath = path.join(this.projectRoot, file.path);
    const fileItem = this.ctrl.createTestItem(
      file.id,
      file.name,
      vscode.Uri.file(absPath),
    );
    this.fileItems.set(file.id, fileItem);
    suiteItem.children.add(fileItem);
    // No populateTestItems here: the snapshot may carry stale messages
    // from a previous run. Children are populated on file_finished.
  }

  private populateTestItems(
    fileItem: vscode.TestItem,
    file: WireFile,
  ): void {
    fileItem.children.replace([]);
    for (let i = 0; i < file.messages.length; i++) {
      const msg = file.messages[i];
      const label = msg.test_name ?? `event ${i + 1}`;
      const id = `${file.id}:${i}`;

      let uri: vscode.Uri | undefined;
      if (msg.location?.line) {
        uri = vscode.Uri.file(
          path.join(this.projectRoot, msg.location.file),
        );
      }
      const testItem = this.ctrl.createTestItem(id, label, uri);
      if (msg.location?.line) {
        testItem.range = new vscode.Range(
          msg.location.line - 1, 0,
          msg.location.line - 1, 0,
        );
      }
      fileItem.children.add(testItem);
    }
  }

  // ── SSE event handling ──────────────────────────────────────────────

  private onEvent(e: SseEvent): void {
    switch (e.kind) {
      case "run_started":
        this.onRunStarted(e.data);
        break;
      case "file_started":
        this.onFileStarted(e.data);
        break;
      case "file_finished":
        this.onFileFinished(e.data);
        break;
      case "run_complete":
      case "run_cancelled":
        this.onRunDone();
        break;
    }
  }

  private onRunStarted(data: any): void {
    if (this.currentRun) {
      this.currentRun.end();
      this.currentRun = null;
    }

    this.currentRun = this.ctrl.createTestRun(
      new vscode.TestRunRequest(),
      `Run ${data.run_id}`,
      false,
    );
    for (const fileId of data.files ?? []) {
      const item = this.fileItems.get(fileId);
      if (item) this.currentRun.enqueued(item);
    }
  }

  private onFileStarted(data: any): void {
    const item = this.fileItems.get(data.file_id);
    if (item && this.currentRun) {
      this.currentRun.started(item);
    }
  }

  private onFileFinished(data: any): void {
    const file: WireFile = data.file;
    const fileItem = this.fileItems.get(file.id);
    if (!fileItem || !this.currentRun) return;

    this.populateTestItems(fileItem, file);

    for (let i = 0; i < file.messages.length; i++) {
      const msg = file.messages[i];
      const testItem = fileItem.children.get(`${file.id}:${i}`);
      if (!testItem) continue;
      this.reportOutcome(testItem, msg);
    }

    if (file.bad) {
      const firstFail = file.messages.find(
        (m) => m.outcome === "fail" || m.outcome === "error",
      );
      const testMsg = new vscode.TestMessage(
        firstFail?.message ?? "test failed",
      );
      if (firstFail?.location?.line && fileItem.uri) {
        testMsg.location = new vscode.Location(
          fileItem.uri,
          new vscode.Position(firstFail.location.line - 1, 0),
        );
      }
      this.currentRun.failed(fileItem, testMsg, file.last_duration_ms);
    } else {
      this.currentRun.passed(fileItem, file.last_duration_ms);
    }
  }

  private reportOutcome(item: vscode.TestItem, msg: WireMessage): void {
    if (!this.currentRun) return;

    const loc =
      msg.location?.line && item.uri
        ? new vscode.Location(
            item.uri,
            new vscode.Position(msg.location.line - 1, 0),
          )
        : undefined;

    switch (msg.outcome) {
      case "pass":
      case "xfail":
        this.currentRun.passed(item, msg.duration_ms);
        break;
      case "fail":
      case "error": {
        const m = new vscode.TestMessage(msg.message ?? msg.outcome);
        if (loc) m.location = loc;
        this.currentRun.failed(item, m, msg.duration_ms);
        break;
      }
      case "warn": {
        const m = new vscode.TestMessage(msg.message ?? "warning");
        if (loc) m.location = loc;
        this.currentRun.failed(item, m, msg.duration_ms);
        break;
      }
      case "skip":
        this.currentRun.skipped(item);
        break;
    }
  }

  private onRunDone(): void {
    this.currentRun?.end();
    this.currentRun = null;
  }

  // ── Run requests from the Test Explorer UI ──────────────────────────

  private async handleRunRequest(
    request: vscode.TestRunRequest,
    token: vscode.CancellationToken,
  ): Promise<void> {
    token.onCancellationRequested(() => {
      this.postJson("/api/cancel", {});
    });

    if (request.include && request.include.length > 0) {
      const fileIds: string[] = [];
      for (const item of request.include) {
        if (item.id.startsWith("suite:")) {
          item.children.forEach((child) => fileIds.push(child.id));
        } else if (item.id.includes(":")) {
          fileIds.push(item.id.split(":")[0]);
        } else {
          fileIds.push(item.id);
        }
      }
      const unique = [...new Set(fileIds)];
      await this.postJson("/api/rerun", { files: unique });
    } else {
      await this.postJson("/api/run", {});
    }
  }

  private postJson(endpoint: string, body: any): Promise<void> {
    return new Promise((resolve) => {
      const data = JSON.stringify(body);
      const u = new URL(endpoint, this.baseUrl);
      const req = http.request(
        {
          hostname: u.hostname,
          port: u.port,
          path: u.pathname,
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "Content-Length": String(Buffer.byteLength(data)),
          },
        },
        () => resolve(),
      );
      req.on("error", () => resolve());
      req.write(data);
      req.end();
    });
  }

  dispose(): void {
    for (const d of this.disposables) d.dispose();
    this.ctrl.dispose();
  }
}
