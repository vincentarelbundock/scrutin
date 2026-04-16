import * as vscode from "vscode";
import * as http from "http";
import type { WireSnapshot, WireFile, WireEventKind } from "./types";

export interface SseEvent {
  kind: WireEventKind;
  data: any;
}

export class EventBus {
  private _onEvent = new vscode.EventEmitter<SseEvent>();
  readonly onEvent = this._onEvent.event;

  private _snapshot: WireSnapshot | null = null;
  private _files: Map<string, WireFile> = new Map();

  get snapshot(): WireSnapshot | null {
    return this._snapshot;
  }

  get files(): ReadonlyMap<string, WireFile> {
    return this._files;
  }

  setSnapshot(snap: WireSnapshot): void {
    this._snapshot = snap;
    this._files.clear();
    for (const f of snap.files) this._files.set(f.id, f);
  }

  updateFile(file: WireFile): void {
    this._files.set(file.id, file);
  }

  fire(event: SseEvent): void {
    if (event.kind === "file_finished" && event.data.file) {
      this.updateFile(event.data.file);
    }
    this._onEvent.fire(event);
  }

  dispose(): void {
    this._onEvent.dispose();
  }
}

export function fetchSnapshot(baseUrl: string): Promise<WireSnapshot> {
  return new Promise((resolve, reject) => {
    http
      .get(`${baseUrl}/api/snapshot`, (res) => {
        if (res.statusCode !== 200) {
          res.resume();
          reject(new Error(`Snapshot fetch failed: HTTP ${res.statusCode}`));
          return;
        }
        let body = "";
        res.on("data", (chunk: Buffer) => {
          body += chunk.toString();
        });
        res.on("end", () => {
          try {
            resolve(JSON.parse(body));
          } catch (e) {
            reject(e);
          }
        });
      })
      .on("error", reject);
  });
}
