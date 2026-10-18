# Compiler diagnostic fixes

This fork keeps a small set of build-log diagnostic fixes on top of
[latex-lsp/texlab](https://github.com/latex-lsp/texlab). The initial upstream base
is `94e6c094ae628125073b2cf70c11dddaaa092140` (version 5.26.0).

The changes cover log parsing and diagnostic source ranges:

- Read explicit `file:line: message` errors from `-file-line-error` logs, including
  wrapped headers, Windows paths, quoted paths and filenames with parentheses.
  Explicit locations take precedence over the traditional log file stack.
- Recognize balanced parentheses in traditional log filenames. Keep consecutive
  errors separate so an error without a line number cannot borrow a later
  error's location. Do not discard errors following lines ending in ellipses or
  lines at the default print width.
- For an undefined control sequence with unambiguous source context, select the
  command token rather than its entire preceding context. Ambiguous or unmatched
  context falls back to the compiler's reported line.
- Keep diagnostics whose source file is unavailable or whose line is out of
  range in the build log, with the reported file/line included in the message.
  Do not substitute the root source file's first line.
- Resolve parsed log errors against the current source text and project graph
  when publishing diagnostics. Source edits no longer reuse old byte ranges,
  and changing the log directory excludes diagnostics from orphaned logs.
  Prefer an existing log in the configured directory over fallback logs left
  behind by older builds; retain fallback discovery if that log is absent.

## Build and try

```sh
git clone --branch fix/build-log-diagnostic-locations https://github.com/Jacky-Lzx/texlab.git
cd texlab
cargo build --release --locked -p texlab
```

Point your editor at this build explicitly while testing. For Neovim's native
LSP configuration, merge the following into your existing texlab configuration:

```lua
return {
  cmd = { "/absolute/path/to/texlab/target/release/texlab" },
  settings = {
    texlab = {
      build = {
        executable = "latexmk",
        args = {
          "-pdf", "-synctex=1", "-interaction=nonstopmode",
          "-file-line-error", "%f",
        },
      },
    },
  },
}
```

Restart the language server and rebuild the document to replace old diagnostics.
Compiler warnings continue to use the traditional log parser.

## Limits

TeX's traditional log syntax cannot reliably distinguish a file-stack closing
parenthesis from arbitrary output such as `\typeout{)}`. Use `-file-line-error`
with this fork for explicit error file/line locations. This option is not a
workaround for the unpatched 5.26.0 server, which misses these errors.

TeX does not always report the original cause of an error or a precise column.
Macro expansion, repeated context and error recovery can still require reading
the log and fixing the first error before rebuilding. The token refinement is
intentionally restricted to undefined control sequences; other error types keep
their context or line location.

## Validation

```sh
cargo test --workspace --all-features --locked
rustfmt --edition 2024 --check crates/parser/src/build_log.rs crates/diagnostics/src/build_log.rs crates/diagnostics/src/manager.rs crates/base-db/src/deps/graph.rs
```

The focused regressions are in `crates/parser/src/build_log/tests.rs` and
`crates/diagnostics/src/build_log/tests.rs`. They cover explicit and traditional
locations, path quoting/parentheses, Windows paths, UTF-8 and tab context,
ambiguous/truncated hints, invalid line numbers, unresolved source locations,
source edits and changes to the configured log directory.

Local validation passed all 403 workspace tests, including 24 added regressions.
A real Neovim/latexmk run checked 14 cases: 11 exact undefined-command ranges and
quickfix positions (including UTF-16 to UTF-8 conversion), missing-dollar and
missing-brace smoke cases, and reproduction of the traditional-log ambiguity
described above. The UTF-8 source case used XeLaTeX; the other compiler cases
used pdfLaTeX. A separate real-build integration passed error clearing after
source edits and output-directory changes, plus PDF-open/forward-search argument
checks using fake viewer processes. GUI viewer and inverse-search behavior were
not validated.
