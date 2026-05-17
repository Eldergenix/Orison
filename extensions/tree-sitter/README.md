# tree-sitter-orison

TreeSitter grammar for the [Orison programming language](https://github.com/Eldergenix/Orison).

It targets the bootstrap-parseable subset shipping today: module decl, imports, record/variant/newtype declarations, services, views, actors, queries, migrations, capabilities, protocols, impls, function bodies with the full operator precedence ladder from `crates/ori-compiler/src/expr_ops.rs` (`||`, `&&`, `== !=`, `< <= > >=`, `+ -`, `* / %`, `??`), record construction, list literals, string interpolation, and numeric literals (decimal / hex / bin / oct / float, all underscore-friendly).

## Layout

```
extensions/tree-sitter/
  grammar.js              ← grammar definition
  package.json            ← npm metadata (consumed by tree-sitter CLI)
  test/corpus/basic.txt   ← golden parse tree fixtures
```

## Building

```sh
cd extensions/tree-sitter
npm install
npx tree-sitter generate
npx tree-sitter test
```

## Embedding in editors

- **Neovim** — drop `parser/orison.so` (after `tree-sitter build`) into `~/.config/nvim/parser/` and register the scope `source.orison` against `.ori`.
- **Helix** — add a `[[language]]` block for `orison` in `languages.toml` with `roots = ["ori.toml"]` and `auto-format = false`.
- **Zed** — point an extension at `extensions/tree-sitter/` directly.

## Precedence + associativity

| Level | Operators | Associativity |
| ----- | --------- | ------------- |
| 1     | `\|\|`    | left          |
| 2     | `&&`      | left          |
| 3     | `==` `!=` | left          |
| 4     | `<` `<=` `>` `>=` | left  |
| 5     | `+` `-`   | left          |
| 6     | `*` `/` `%` | left        |
| 7     | `??`      | right         |
| 8     | unary `-` `!` `await` | prefix |
| 9     | call      | postfix       |
| 10    | `.` `[]`  | postfix       |
| 11    | `?` (try) | postfix       |

These mirror the Pratt loop in `crates/ori-compiler/src/expr_ops.rs` — any change there must be mirrored here.

## Known limitations

- The grammar is **indentation-tolerant**, not indentation-driven. The bootstrap compiler computes layout from significant whitespace; TreeSitter accepts the same source but does not enforce it. The LSP semantic-tokens path remains the source of truth for layout-sensitive highlighting.
- `view`-block UI primitives (`list:`, `card:`, `form:`, `heading(...)`, etc.) parse as ordinary block / call expressions; structured view DSL nodes are deferred to a follow-up.

## License

Apache-2.0.
