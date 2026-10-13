import * as path from 'path';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: { include: ['src/test/unit/**/*.test.ts', 'src/test/integration/**/*.test.ts'] },
  resolve: {
    // The real `vscode` module only exists inside the editor.
    alias: { vscode: path.resolve(__dirname, 'src/test/mocks/vscode.ts') },
  },
});
