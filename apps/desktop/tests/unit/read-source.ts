import { readFileSync } from 'node:fs'

const SHELL_PREFIXES: [string, string][] = [
  ['src/', 'apps/desktop/src/'],
  ['src-tauri/', 'apps/desktop/src-tauri/'],
]

function repositoryPath(relativePath: string): string {
  const prefix = SHELL_PREFIXES.find(([from]) => relativePath.startsWith(from))
  return prefix ? `${prefix[1]}${relativePath.slice(prefix[0].length)}` : relativePath
}

export function readSource(relativePath: string): string {
  const source = readFileSync(new URL(`../../../../${repositoryPath(relativePath)}`, import.meta.url), 'utf8')
  return source.replace(/\r\n/g, '\n')
}

export function readLocale(name: string): Record<string, string> {
  return JSON.parse(readSource(`src/i18n/locales/${name}.json`)) as Record<string, string>
}
