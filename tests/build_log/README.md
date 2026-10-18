# Real LaTeX build-log corner cases

This suite compiles the TeX sources in `cases.json` with real `latexmk` /
pdfLaTeX, starts the selected texlab build through Neovim's LSP client, and
checks the published compiler diagnostics. It does not depend on tex-tools.nvim
or a personal Neovim configuration.

## Run

Requires Neovim 0.11 or newer, `latexmk` and pdfLaTeX with the standard LaTeX
article class. No CJK packages or GUI tools are required: the Unicode prefix is
inside `\iffalse ... \fi`, so pdfLaTeX can skip it while the test still checks
UTF-16 positions after non-BMP and CJK characters.

From the texlab checkout:

```sh
cargo build --locked -p texlab
TEXLAB_BINARY="$PWD/target/debug/texlab" \
  TEXLAB_TEST_OUTPUT="$(mktemp -d)" \
  nvim --clean --headless -i NONE -l tests/build_log/run.lua
```

`TEXLAB_TEST_OUTPUT` must be empty. If omitted, a new directory is created under
the system temporary directory. The runner retains generated sources, compiler
stdout/stderr (`compiler.txt`), TeX logs, recorder files and `results.json`.
Set `TEXLAB_TEST_FILTER=dot-segments` to select names containing that text.

The runner disables latexmk rc files with `-norc`, gives each case a separate
project root, waits for diagnostics to settle, and returns nonzero for a
compiler/LSP failure or an unexpected diagnostic result.

## Coverage

There are 39 scenarios, each tested with both traditional errors and
`-file-line-error`, giving 78 compiler/LSP cases:

- Multiple errors on one line and on separate lines; repeated truncated
  context must fall back to a line without guessing a command occurrence.
- Macro expansion, nested macros, `\edef` and multiline macro arguments.
  A macro's call site can be identified, but a multiline argument may only
  support the compiler's reported line, not the original undefined token.
- Control symbols, `@` commands, internal tabs, repeated spaces, literal
  ellipses, commented commands, UTF-16 positions after non-BMP/CJK text,
  CRLF input and source without a final newline.
- Narrow and wide error context, exactly 79-column context, context wrapped
  in the middle of a command, and long wrapped error file paths.
- Nested siblings with the same basename, `\include`, spaces/parentheses in
  directory names, Unicode filenames, `..` segments, a root source in a
  subdirectory, `latexmk -cd`, `.def` / `.ltx` / uppercase `.TEX` inputs and
  package errors.
- Missing input files with emergency-stop/fatal diagnostics, missing-dollar
  recovery, multiline package errors, package warnings and citation warnings.

Each expectation checks message, severity and the total diagnostic count. File
and line are checked where specified. Undefined commands additionally require
an exact UTF-16 token range or an empty range at the reported line's start.
Other error types and warnings do not promise an exact column.

The runner opens every fixture source as LaTeX before checking diagnostics,
including unusual extensions. This tests parsing and location resolution for
loaded documents; filetype detection and discovery of unopened files are
separate concerns.

Windows paths, URI-reserved characters and further file extensions are covered
by the Rust regressions without requiring a Windows TeX installation.

## Explicitly tracked limitations

Two traditional-log cases are marked `XFAIL`:

1. `\typeout{)}` looks like a file-stack closing marker.
2. A filename containing an unmatched `)` is printed without quotes by
   pdfLaTeX, even when the `\input` argument is quoted.

Both corresponding `-file-line-error` cases must pass. An `XFAIL` still runs and
checks the correct source expectations; its mismatches and reason remain in
`results.json`. An unexpected pass (`XPASS`) fails the suite so the limitation
annotation must be removed when the behavior improves. Compiler and LSP
failures cannot be treated as these known limitations.

## Results from this repair round

The Rust workspace passes 420 tests, including 17 regressions added in this
round. The compiler/LSP suite has 76 passing cases and the two tracked `XFAIL`
cases above, with no unexpected failures. Both explicit-location counterparts
of those `XFAIL` cases pass.

Validation used Neovim 0.12.6, latexmk 4.87, pdfTeX 1.40.27 (TeX Live 2025) and
Rust 1.98.1 on Linux. The separate tex-tools.nvim real-build integration also
passes error clearing after source/output-directory changes and viewer-argument
checks. GUI viewer and inverse-search behavior are outside these checks.

The added cases found and now cover these omissions in the initial repair:

| Corner case | Initial behavior | Corrected behavior |
| --- | --- | --- |
| Main source under `src/`, compiler launched at project root | Relative log paths were joined to `src/` again; diagnostic fell back to the log | Resolve against the project compile directory |
| `latexmk -cd` | The inner compiler changes directory, so the project directory is also insufficient | Prefer the companion `.fls` recorder's absolute `PWD`; fall back if absent/invalid |
| `chapters/../shared/child.tex` | The path did not match the normalized source URI | Normalize directory segments while preserving literal URI-reserved filename characters |
| Traditional `.def` / `.ltx` / uppercase `.TEX` inputs | Their errors were attributed to the enclosing `.tex` file | Recognize standard TeX extensions without case sensitivity |
