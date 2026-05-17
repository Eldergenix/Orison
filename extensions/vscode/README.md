# Orison Language Support for VS Code

Official Visual Studio Code extension for the [Orison programming language](https://github.com/Eldergenix/Orison).

It ships:

- TextMate-based syntax highlighting for `.ori` files (keywords, types, variants, string interpolation, numeric literals with underscores, `Result[List[Option[T]], E]`-style nested generics).
- A thin language-client that talks to the official `ori lsp --stdio` server. The server provides hover, completion, rename, code actions, workspace and document symbols, go-to-definition, find-all-references, and more (see [`crates/ori-lsp`](https://github.com/Eldergenix/Orison/tree/main/crates/ori-lsp)).
- Editor commands for restarting the server, running the current file, and opening the doctor report.

## Requirements

- VS Code 1.85.0 or newer.
- The `ori` toolchain on your `PATH`, or an explicit absolute path configured via `orison.serverPath`. Install instructions live at the repo root.

## Installation

### From a `.vsix`

```sh
cd extensions/vscode
npm install
npm run build
npx --yes @vscode/vsce package --no-yarn
code --install-extension orison-language-support-0.1.0.vsix
```

### From source (development)

```sh
cd extensions/vscode
npm install
npm run watch
```

Then press `F5` in VS Code with the `extensions/vscode/` folder open to launch an Extension Development Host.

## Configuration

| Setting | Default | Description |
| --- | --- | --- |
| `orison.serverPath` | `""` | Absolute path to the `ori` binary. Empty means "fall back to `ori` on `PATH`". |
| `orison.trace.server` | `"off"` | LSP trace level. One of `off`, `messages`, `verbose`. |

## Commands

| Command | ID |
| --- | --- |
| Orison: Restart Language Server | `orison.restartLsp` |
| Orison: Open Doctor Report | `orison.openDoctor` |
| Orison: Run Current File | `orison.runCurrentFile` |

## Troubleshooting

1. **"Orison: failed to start language server"** — confirm `ori --version` works in a terminal. If you keep `ori` outside `PATH`, set `orison.serverPath` to its absolute location.
2. **No diagnostics show up** — set `orison.trace.server` to `verbose` and inspect the `Orison` output channel; the LSP server prints every initialize / didOpen exchange there.
3. **Highlighting looks off** — check the file extension is `.ori` and the language mode (bottom-right corner) is `Orison`. The grammar lives at `syntaxes/orison.tmLanguage.json` if you want to tinker.
4. **Restart doesn't pick up a new server binary** — run the `Orison: Restart Language Server` command, or reload the window with `Developer: Reload Window`.

## License

Apache-2.0, same as the parent project.
