import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'
import { getDefaultConfig } from 'expo/metro-config.js'
import { withUniwindConfig } from 'uniwind/metro'

const rootDir = path.dirname(fileURLToPath(import.meta.url))
const config = withUniwindConfig(getDefaultConfig(rootDir), {
  cssEntryFile: './src/global.css',
  dtsFile: './src/uniwind-types.d.ts',
})
const upstreamResolveRequest = config.resolver.resolveRequest
config.resolver.resolveRequest = (context, moduleName, platform) => {
  const resolveRequest = upstreamResolveRequest ?? context.resolveRequest
  if (moduleName === '@' || moduleName.startsWith('@/'))
    return resolveRequest(context, path.join(rootDir, 'src', moduleName.slice(2)), platform)
  return resolveRequest(context, moduleName, platform)
}
const blockedRootDirs = ['.git', '.temp', '.tmp', '.worktrees', 'dist']
config.resolver.blockList = [
  ...[config.resolver.blockList].flat().filter(Boolean),
  ...blockedRootDirs.map(dir => new RegExp(`^${path.join(rootDir, dir).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[\\/]`)),
],
if (process.platform === 'win32') config.maxWorkers = 2
export default config
