// feat: VSCode client for the Slab language server (`slab lsp` over stdio) with live SVG preview.
import { randomBytes } from "node:crypto";
import * as vscode from "vscode";
import {
  Executable,
  LanguageClient,
  LanguageClientOptions,
} from "vscode-languageclient/node";

/** Diagnostic entry returned by the custom `slab/preview` request. */
interface PreviewDiag {
  level: string;
  code: string;
  msg: string;
  line: number;
}

/** Result payload of the custom `slab/preview` request. */
interface PreviewResult {
  svg: string;
  width: number;
  height: number;
  diags: PreviewDiag[];
}

/** Per-document preview panel state; `width === null` tracks the panel (auto). */
interface Preview {
  panel: vscode.WebviewPanel;
  width: number | null;
  measured?: number;
  last?: PreviewResult;
}

/** Active language client, undefined while stopped or disabled. */
let client: LanguageClient | undefined;

/** Tracks whether the spawn-failure warning was already shown this session. */
let warnedSpawnFailure = false;

/** Open preview panels keyed by document URI. */
const previews = new Map<string, Preview>();

/** Pending debounced preview refreshes keyed by document URI. */
const refreshTimers = new Map<string, NodeJS.Timeout>();

/** Starts the language client; warns once (non-fatal) if the server cannot spawn. */
async function startClient(): Promise<void> {
  const config = vscode.workspace.getConfiguration("slab");
  if (!config.get<boolean>("lsp.enabled", true)) return;
  const argv = config.get<string[]>("lsp.command", ["slab", "lsp"]);
  if (!Array.isArray(argv) || argv.length === 0) return;

  const server: Executable = {
    command: argv[0],
    args: argv.slice(1),
    options: { cwd: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "slab" }],
  };
  const next = new LanguageClient("slab", "Slab Language Server", server, clientOptions);
  try {
    await next.start();
    client = next;
  } catch (err) {
    client = undefined;
    if (!warnedSpawnFailure) {
      warnedSpawnFailure = true;
      vscode.window.showWarningMessage(
        `Slab: could not start language server (${server.command}): ${String(err)}. ` +
          "Syntax highlighting still works; check the `slab.lsp.command` setting."
      );
    }
  }
}

/** Stops the language client if it is running. */
async function stopClient(): Promise<void> {
  if (!client) return;
  const stopping = client;
  client = undefined;
  await stopping.stop().catch(() => undefined);
}

/**
 * Builds the webview skeleton once per panel. Renders stream in afterwards via
 * postMessage, so scroll position and toolbar state survive refreshes and the
 * ResizeObserver keeps auto width in sync with the panel.
 */
function previewHtml(): string {
  const nonce = randomBytes(16).toString("base64");
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'nonce-${nonce}'">
<style>
  html { scrollbar-gutter: stable; }
  body { margin: 0; font-family: var(--vscode-font-family); color: var(--vscode-foreground); }
  .toolbar { display: flex; align-items: center; gap: 6px; padding: 6px 10px; position: sticky; top: 0; z-index: 1;
    background: var(--vscode-editor-background); border-bottom: 1px solid var(--vscode-widget-border, #0003); }
  .toolbar button { cursor: pointer; }
  .toolbar button.active { outline: 1px solid var(--vscode-focusBorder, #07f); }
  .toolbar input { width: 64px; }
  .dims { opacity: 0.7; margin-left: auto; font-size: 11px; }
  main { padding: 16px; display: flex; justify-content: center; }
  .stage { display: inline-block; line-height: 0;
    background: repeating-conic-gradient(#8882 0% 25%, transparent 0% 50%) 0 0 / 16px 16px, #80808014; }
  .empty { opacity: 0.8; line-height: 1.4; }
  .diags { padding: 0 16px 16px; }
  .diags ul { list-style: none; padding: 0; margin: 0; font-size: 12px; }
  .diag { padding: 2px 0; }
  .diag .code { font-weight: 600; }
  .diag-error .code { color: var(--vscode-errorForeground, #f66); }
  .diag-warning .code { color: var(--vscode-editorWarning-foreground, #fc6); }
  .diag-note .code { opacity: 0.75; }
  .diag .line { opacity: 0.6; }
</style>
</head>
<body>
<div class="toolbar">
  <button data-zoom="fit" class="active">Fit</button>
  <button data-zoom="100">100%</button>
  <button data-zoom="200">200%</button>
  <label>w <input id="width" type="number" min="1" placeholder="auto" title="Solve width in u; empty tracks the panel"></label>
  <span class="dims" id="dims"></span>
</div>
<main><div class="stage" id="stage"><p class="empty">Rendering\u2026</p></div></main>
<section class="diags"><ul id="diags"></ul></section>
<script nonce="${nonce}">
  const vscode = acquireVsCodeApi();
  const stage = document.getElementById("stage");
  const dims = document.getElementById("dims");
  const diagList = document.getElementById("diags");
  const widthInput = document.getElementById("width");
  let zoom = "fit";
  let docWidth = 0;

  function applyZoom() {
    const svg = stage.querySelector("svg");
    if (!svg) return;
    if (zoom === "fit") { svg.style.maxWidth = "100%"; svg.style.width = ""; }
    else { svg.style.maxWidth = ""; svg.style.width = (docWidth * (zoom === "200" ? 2 : 1)) + "px"; }
    svg.style.height = "auto";
  }

  for (const b of document.querySelectorAll("button[data-zoom]")) {
    b.addEventListener("click", () => {
      zoom = b.dataset.zoom;
      for (const o of document.querySelectorAll("button[data-zoom]")) o.classList.toggle("active", o === b);
      applyZoom();
    });
  }

  widthInput.addEventListener("change", () => {
    const value = Number(widthInput.value);
    vscode.postMessage({ type: "width", value: widthInput.value === "" ? null :
      (Number.isFinite(value) && value > 0 ? value : null) });
  });

  // Auto width: report the stage's available width whenever the panel resizes.
  let measureTimer;
  const observer = new ResizeObserver(() => {
    clearTimeout(measureTimer);
    measureTimer = setTimeout(() => {
      const w = Math.floor(document.body.clientWidth) - 32; // main padding
      if (w > 0) vscode.postMessage({ type: "measure", value: w });
    }, 100);
  });
  observer.observe(document.body);

  window.addEventListener("message", (event) => {
    const msg = event.data;
    if (msg.type !== "render") return;
    docWidth = msg.width || 0;
    if (msg.svg) {
      stage.innerHTML = msg.svg;
      applyZoom();
      dims.textContent = Math.round(msg.width) + "\\u00d7" + Math.round(msg.height);
    } else {
      const p = document.createElement("p");
      p.className = "empty";
      p.textContent = msg.error || "No render \\u2014 see diagnostics below.";
      stage.replaceChildren(p);
      dims.textContent = "";
    }
    diagList.replaceChildren(...(msg.diags || []).map((d) => {
      const li = document.createElement("li");
      li.className = "diag diag-" + d.level;
      const code = document.createElement("span");
      code.className = "code";
      code.textContent = d.level + "/" + d.code;
      const line = document.createElement("span");
      line.className = "line";
      line.textContent = " L" + d.line + " ";
      li.append(code, line, d.msg);
      return li;
    }));
  });
</script>
</body>
</html>`;
}

/** Requests a render at the effective width (manual override or measured panel width). */
async function refreshPreview(uri: string): Promise<void> {
  const preview = previews.get(uri);
  if (!preview) return;
  const post = (msg: object) => void preview.panel.webview.postMessage(msg);
  if (!client) {
    post({ type: "render", svg: "", diags: [], error: "Slab language server is not running." });
    return;
  }
  try {
    preview.last = await client.sendRequest<PreviewResult>("slab/preview", {
      uri,
      width: preview.width ?? preview.measured,
    });
    post({ type: "render", ...preview.last });
  } catch (err) {
    post({ type: "render", svg: "", diags: [], error: `Preview request failed: ${String(err)}` });
  }
}

/** Debounces a preview refresh for the given document URI (~150 ms). */
function scheduleRefresh(uri: string): void {
  clearTimeout(refreshTimers.get(uri));
  refreshTimers.set(
    uri,
    setTimeout(() => {
      refreshTimers.delete(uri);
      void refreshPreview(uri);
    }, 150)
  );
}

/** Opens (or reveals) the preview panel for a document and wires its messages. */
function openPreview(document: vscode.TextDocument): void {
  if (!client) {
    vscode.window.showWarningMessage(
      "Slab: preview needs the language server; enable `slab.lsp.enabled` or fix `slab.lsp.command`."
    );
    return;
  }
  const uri = document.uri.toString();
  const existing = previews.get(uri);
  if (existing) {
    existing.panel.reveal(undefined, true);
    return;
  }
  const shortName = document.uri.path.split("/").pop() ?? "slab";
  const panel = vscode.window.createWebviewPanel(
    "slabPreview",
    `Preview ${shortName}`,
    { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
    { enableScripts: true, retainContextWhenHidden: true }
  );
  const preview: Preview = { panel, width: null };
  previews.set(uri, preview);
  panel.onDidDispose(() => {
    previews.delete(uri);
    clearTimeout(refreshTimers.get(uri));
    refreshTimers.delete(uri);
  });
  panel.webview.onDidReceiveMessage((msg: unknown) => {
    if (!msg || typeof msg !== "object" || !("type" in msg)) return;
    if (msg.type === "width" && "value" in msg) {
      preview.width = typeof msg.value === "number" && msg.value > 0 ? msg.value : null;
      void refreshPreview(uri);
    } else if (msg.type === "measure" && "value" in msg && typeof msg.value === "number") {
      const changed = preview.measured === undefined || Math.abs(preview.measured - msg.value) >= 1;
      preview.measured = msg.value;
      if (preview.width === null && changed) scheduleRefresh(uri);
    }
  });
  panel.webview.html = previewHtml();
  // First render waits for the initial measure; this fallback covers webviews
  // that report no resize (hidden panel) so something always appears.
  setTimeout(() => {
    if (!preview.last) void refreshPreview(uri);
  }, 300);
}

/** Extension entry point: registers commands, change listeners, and starts the LSP client. */
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand("slab.restartLsp", async () => {
      warnedSpawnFailure = false;
      await stopClient();
      await startClient();
      for (const uri of previews.keys()) scheduleRefresh(uri);
    }),
    vscode.commands.registerCommand("slab.preview", () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "slab") {
        vscode.window.showWarningMessage("Slab: open a .slab document to preview it.");
        return;
      }
      openPreview(editor.document);
    }),
    vscode.workspace.onDidChangeTextDocument((e) => {
      const uri = e.document.uri.toString();
      if (previews.has(uri)) scheduleRefresh(uri);
    }),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration("slab.lsp")) {
        await stopClient();
        await startClient();
      }
    })
  );
  await startClient();
}

/** Extension teardown: stops the language client and closes preview panels. */
export async function deactivate(): Promise<void> {
  for (const preview of previews.values()) preview.panel.dispose();
  previews.clear();
  await stopClient();
}
