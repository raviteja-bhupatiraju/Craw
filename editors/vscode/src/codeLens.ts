export interface LensSpec {
  line: number;
  title: string;
  command: string;
}

/**
 * The run lenses for a Craw file. A Craw program runs top to bottom, so
 * the lenses sit on the first line of code (after any leading comments
 * and blank lines, which usually hold a file header).
 */
export function runLenses(text: string): LensSpec[] {
  const lines = text.split(/\r?\n/);
  const first = lines.findIndex((l) => l.trim() !== '' && !l.trimStart().startsWith('#'));
  if (first < 0) return [];
  return [
    { line: first, title: '▶ Run', command: 'craw.run' },
    { line: first, title: 'Build', command: 'craw.build' },
    { line: first, title: 'Show Rust', command: 'craw.showTranspiledRust' },
  ];
}
