import * as vscode from "vscode";
import { ChildProcess, spawn } from "child_process";
import * as http from "http";
import * as net from "net";

export class ScrutinProcess {
  private child: ChildProcess | null = null;
  private _baseUrl: string | null = null;
  private _onExit = new vscode.EventEmitter<number | null>();
  readonly onExit = this._onExit.event;

  get baseUrl(): string | null {
    return this._baseUrl;
  }

  get running(): boolean {
    return this.child !== null && this.child.exitCode === null;
  }

  async start(
    binaryPath: string,
    projectRoot: string,
  ): Promise<string> {
    if (this.running) {
      return this._baseUrl!;
    }

    const port = await findFreePort();
    const args = ["-r", `web:127.0.0.1:${port}`, projectRoot];

    this.child = spawn(binaryPath, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });

    const url = await this.waitForReady(port);
    this._baseUrl = url;

    this.child.on("exit", (code) => {
      this.child = null;
      this._baseUrl = null;
      this._onExit.fire(code);
    });

    return url;
  }

  stop(): void {
    if (this.child) {
      this.child.kill("SIGTERM");
      this.child = null;
      this._baseUrl = null;
    }
  }

  private waitForReady(expectedPort: number): Promise<string> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error("scrutin server did not start within 10 seconds"));
      }, 10_000);

      const stderr = this.child?.stderr;
      if (!stderr) {
        clearTimeout(timeout);
        reject(new Error("no stderr stream"));
        return;
      }

      let buf = "";
      const onData = (chunk: Buffer) => {
        buf += chunk.toString();
        const match = buf.match(/scrutin-web: listening on (http:\/\/[^\s]+)/);
        if (match) {
          clearTimeout(timeout);
          stderr.removeListener("data", onData);
          resolve(match[1]);
        }
      };
      stderr.on("data", onData);

      this.child?.on("exit", (code) => {
        clearTimeout(timeout);
        stderr.removeListener("data", onData);
        const tail = buf.trim().split("\n").slice(-5).join("\n");
        const detail = tail ? `: ${tail}` : "";
        reject(new Error(`scrutin exited with code ${code} before ready${detail}`));
      });
    });
  }

  dispose(): void {
    this.stop();
    this._onExit.dispose();
  }
}

function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address() as net.AddressInfo;
      srv.close(() => resolve(addr.port));
    });
    srv.on("error", reject);
  });
}
