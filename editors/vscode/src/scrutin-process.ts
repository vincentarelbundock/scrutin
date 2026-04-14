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
    watch: boolean,
  ): Promise<string> {
    if (this.running) {
      return this._baseUrl!;
    }

    const port = await findFreePort();
    const args = ["-r", `web:127.0.0.1:${port}`, "--no-open"];
    if (watch) {
      args.push("-w");
    }
    args.push(projectRoot);

    this.child = spawn(binaryPath, args, {
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, CI: "true" }, // suppress browser open
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
        reject(new Error(`scrutin exited with code ${code} before ready`));
      });
    });
  }

  /** Fetch JSON from the scrutin API. */
  async fetchJson<T>(path: string): Promise<T | null> {
    if (!this._baseUrl) return null;
    const url = `${this._baseUrl}${path}`;
    return new Promise((resolve) => {
      http.get(url, (res) => {
        let body = "";
        res.on("data", (c: Buffer) => (body += c.toString()));
        res.on("end", () => {
          try {
            resolve(JSON.parse(body));
          } catch {
            resolve(null);
          }
        });
      }).on("error", () => resolve(null));
    });
  }

  /** POST JSON to the scrutin API. */
  async postJson(path: string, data?: unknown): Promise<void> {
    if (!this._baseUrl) return;
    const url = new URL(path, this._baseUrl);
    const payload = data ? JSON.stringify(data) : undefined;
    return new Promise((resolve) => {
      const req = http.request(
        {
          hostname: url.hostname,
          port: url.port,
          path: url.pathname,
          method: "POST",
          headers: payload
            ? { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(payload) }
            : {},
        },
        () => resolve(),
      );
      req.on("error", () => resolve());
      if (payload) req.write(payload);
      req.end();
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
