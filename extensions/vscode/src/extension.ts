// Orison Language Support — VS Code extension entry point.
//
// Spawns the `ori` binary as a language server using stdio transport and
// wires it to the editor via vscode-languageclient. The server path is
// resolved from `orison.serverPath` and falls back to `ori` on PATH so
// the extension can run out-of-the-box for developers who already have
// the toolchain installed.

import * as path from "node:path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
  Trace,
} from "vscode-languageclient/node";

const LANGUAGE_ID = "orison";
const CLIENT_ID = "orison";
const CLIENT_NAME = "Orison Language Server";
const DEFAULT_COMMAND = "ori";
const LSP_ARGS = ["lsp", "--stdio"];

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  outputChannel = vscode.window.createOutputChannel("Orison");
  context.subscriptions.push(outputChannel);

  context.subscriptions.push(
    vscode.commands.registerCommand("orison.restartLsp", restartLsp),
    vscode.commands.registerCommand("orison.openDoctor", openDoctor),
    vscode.commands.registerCommand("orison.runCurrentFile", runCurrentFile),
  );

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (
        event.affectsConfiguration("orison.serverPath") ||
        event.affectsConfiguration("orison.trace.server")
      ) {
        void restartLsp();
      }
    }),
  );

  await startLsp(context);
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

function resolveServerCommand(): string {
  const configured = vscode.workspace
    .getConfiguration("orison")
    .get<string>("serverPath", "")
    .trim();
  return configured.length > 0 ? configured : DEFAULT_COMMAND;
}

function resolveTraceLevel(): Trace {
  const setting = vscode.workspace
    .getConfiguration("orison")
    .get<string>("trace.server", "off");
  switch (setting) {
    case "verbose":
      return Trace.Verbose;
    case "messages":
      return Trace.Messages;
    default:
      return Trace.Off;
  }
}

async function startLsp(context: vscode.ExtensionContext): Promise<void> {
  const command = resolveServerCommand();

  const serverOptions: ServerOptions = {
    run: { command, args: LSP_ARGS, transport: TransportKind.stdio },
    debug: { command, args: LSP_ARGS, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: LANGUAGE_ID },
      { scheme: "untitled", language: LANGUAGE_ID },
    ],
    synchronize: {
      fileEvents: [
        vscode.workspace.createFileSystemWatcher("**/*.ori"),
        vscode.workspace.createFileSystemWatcher("**/ori.toml"),
      ],
      configurationSection: "orison",
    },
    outputChannel,
    traceOutputChannel: outputChannel,
  };

  client = new LanguageClient(CLIENT_ID, CLIENT_NAME, serverOptions, clientOptions);
  await client.setTrace(resolveTraceLevel());

  try {
    await client.start();
    context.subscriptions.push(client);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    outputChannel?.appendLine(`[orison] failed to start LSP: ${message}`);
    void vscode.window.showErrorMessage(
      `Orison: failed to start language server (${command}). ${message}`,
    );
  }
}

async function restartLsp(): Promise<void> {
  if (!client) {
    return;
  }
  outputChannel?.appendLine("[orison] restarting language server");
  try {
    await client.stop();
  } catch (err) {
    outputChannel?.appendLine(`[orison] stop failed: ${String(err)}`);
  }
  client = undefined;
  // Re-activate via the same code path used at extension load.
  const ext = vscode.extensions.getExtension("Eldergenix.orison-language-support");
  if (ext) {
    await startLsp(ext as unknown as vscode.ExtensionContext);
  }
}

async function openDoctor(): Promise<void> {
  const command = resolveServerCommand();
  const terminal = vscode.window.createTerminal({ name: "Orison Doctor" });
  terminal.show(true);
  terminal.sendText(`${command} doctor`, true);
}

async function runCurrentFile(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== LANGUAGE_ID) {
    void vscode.window.showWarningMessage("Orison: open an .ori file first.");
    return;
  }
  const command = resolveServerCommand();
  const filePath = editor.document.uri.fsPath;
  const cwd = vscode.workspace.getWorkspaceFolder(editor.document.uri)?.uri.fsPath;
  const terminal = vscode.window.createTerminal({
    name: `Orison Run: ${path.basename(filePath)}`,
    cwd,
  });
  terminal.show(true);
  terminal.sendText(`${command} run ${JSON.stringify(filePath)}`, true);
}
