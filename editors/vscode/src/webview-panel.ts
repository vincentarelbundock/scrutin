import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";

export function createPanel(
  context: vscode.ExtensionContext,
  baseUrl: string,
): vscode.WebviewPanel {
  const panel = vscode.window.createWebviewPanel(
    "scrutin",
    "Scrutin",
    vscode.ViewColumn.One,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [
        vscode.Uri.file(path.join(context.extensionPath, "webview")),
      ],
    },
  );

  const webviewDir = path.join(context.extensionPath, "webview");
  const htmlPath = path.join(webviewDir, "index.html");

  // Read the HTML shell template.
  let html = fs.readFileSync(htmlPath, "utf-8");

  // Resolve local resource URIs for the webview.
  const styleUri = panel.webview.asWebviewUri(
    vscode.Uri.file(path.join(webviewDir, "style.css")),
  );
  const scriptUri = panel.webview.asWebviewUri(
    vscode.Uri.file(path.join(webviewDir, "app.js")),
  );

  // Nonce for CSP.
  const nonce = getNonce();

  // Inject URIs, nonce, and CSP into the HTML template.
  html = html
    .replace("{{styleUri}}", styleUri.toString())
    .replace("{{scriptUri}}", scriptUri.toString())
    .replace(/\{\{nonce\}\}/g, nonce)
    .replace("{{baseUrl}}", baseUrl)
    .replace(
      "{{csp}}",
      `default-src 'none'; ` +
        `style-src ${panel.webview.cspSource} 'unsafe-inline'; ` +
        `script-src 'nonce-${nonce}'; ` +
        `connect-src ${baseUrl};`,
    );

  panel.webview.html = html;

  // Handle messages from the webview.
  panel.webview.onDidReceiveMessage(
    async (msg) => {
      if (msg.command === "openFile") {
        const filePath: string = msg.path;
        const line: number | undefined = msg.line;
        try {
          const doc = await vscode.workspace.openTextDocument(filePath);
          const opts: vscode.TextDocumentShowOptions = {};
          if (line != null && line > 0) {
            const pos = new vscode.Position(line - 1, 0);
            opts.selection = new vscode.Range(pos, pos);
          }
          await vscode.window.showTextDocument(doc, opts);
        } catch (e: unknown) {
          const errMsg = e instanceof Error ? e.message : String(e);
          vscode.window.showErrorMessage(`Could not open file: ${errMsg}`);
        }
      }
    },
    undefined,
    context.subscriptions,
  );

  return panel;
}

function getNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let nonce = "";
  for (let i = 0; i < 32; i++) {
    nonce += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return nonce;
}
