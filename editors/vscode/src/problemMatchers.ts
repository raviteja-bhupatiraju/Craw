/**
 * The `$craw` and `$craw-rustc` problem matcher patterns, as plain data —
 * exported so they can be unit-tested here and written verbatim into
 * package.json's `contributes.problemMatchers` (a test asserts the two
 * copies stay identical).
 */

export const crawPattern = {
  regexp: '^(.+?):(\\d+):(\\d+): (error|warning): (.+)$',
  file: 1,
  line: 2,
  column: 3,
  severity: 4,
  message: 5,
};

/**
 * Matches rustc's default human-readable two-line diagnostic form
 * (`error[E0308]: mismatched types` followed by `  --> file.rs:10:5`).
 * `craw build` runs rustc on the generated Rust, so today this points at
 * a line in the *generated* file — mapping it back to the original
 * `.craw` line is `craw check`'s job (not implemented yet).
 */
export const rustcPattern = [
  {
    regexp: '^(warning|error)(\\[E\\d+\\])?: (.*)$',
    severity: 1,
    message: 3,
  },
  {
    regexp: '^\\s*-->\\s(.*):(\\d+):(\\d+)$',
    file: 1,
    line: 2,
    column: 3,
  },
];
