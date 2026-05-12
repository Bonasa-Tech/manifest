import path from 'path';

const TESTS_DIR = path.resolve(__dirname, '..');

function isTestFile(filename: string | undefined): boolean {
  return filename != null && path.resolve(filename).startsWith(TESTS_DIR);
}

export function describeIfDirectTest(
  currentModule: NodeModule,
  title: string,
  fn: (this: Mocha.Suite) => void,
): Mocha.Suite | void {
  if (!isTestFile(currentModule.parent?.filename)) {
    return describe(title, fn);
  }
}
