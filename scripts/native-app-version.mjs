import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { pathToFileURL } from 'node:url'
import { parseSemver } from './release-identity.mjs'

const PACKAGE_JSON = 'package.json'
const APP_JSON = 'app.json'

function nativeAppVersionError(message, cause) {
  const error = new Error(`NATIVE_APP_VERSION: ${message}`)
  if (cause !== undefined)
    error.cause = cause
  return error
}

function readJson(filePath, label) {
  let parsed
  try {
    parsed = JSON.parse(readFileSync(filePath, 'utf8').replace(/^\uFEFF/, ''))
  }
  catch (error) {
    throw nativeAppVersionError(`cannot read ${label} at ${filePath}`, error)
  }

  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
    throw nativeAppVersionError(`${label} must contain a JSON object`)

  return parsed
}

function readDesktopVersion(repo) {
  const file = path.join(repo, PACKAGE_JSON)
  const version = readJson(file, PACKAGE_JSON).version
  if (typeof version !== 'string')
    throw nativeAppVersionError(`${PACKAGE_JSON} version must be a string`)

  try {
    parseSemver(version, `${PACKAGE_JSON} version`)
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw nativeAppVersionError(message.replace(/^RELEASE_IDENTITY: /, ''))
  }

  return version
}

function readExpoConfig(repo) {
  const file = path.join(repo, APP_JSON)
  const app = readJson(file, APP_JSON)
  const expo = app.expo
  if (!expo || typeof expo !== 'object' || Array.isArray(expo))
    throw nativeAppVersionError(`${APP_JSON} must contain the expo object`)
  if (typeof expo.version !== 'string')
    throw nativeAppVersionError(`${APP_JSON} expo.version must be a string`)

  const android = expo.android
  if (!android || typeof android !== 'object' || Array.isArray(android))
    throw nativeAppVersionError(`${APP_JSON} must contain the expo.android object`)
  if (!Number.isInteger(android.versionCode) || android.versionCode < 1)
    throw nativeAppVersionError(`${APP_JSON} expo.android.versionCode must be a positive integer`)

  return { file, app, expo, android }
}

function syncNativeAppVersion({ repo = process.cwd() } = {}) {
  const version = readDesktopVersion(repo)
  const { file, app, expo, android } = readExpoConfig(repo)
  const previousVersion = expo.version
  const previousVersionCode = android.versionCode

  if (previousVersion === version) {
    return { file, changed: false, version, versionCode: previousVersionCode, previousVersion, previousVersionCode }
  }

  // Android 要求 versionCode 严格递增；历史发版实测为每次 version 变化 +1，故不采用编码公式。
  const versionCode = previousVersionCode + 1
  expo.version = version
  android.versionCode = versionCode
  writeFileSync(file, `${JSON.stringify(app, null, 2)}\n`, 'utf8')

  return { file, changed: true, version, versionCode, previousVersion, previousVersionCode }
}

function main() {
  const result = syncNativeAppVersion({ repo: process.cwd() })
  const before = `${result.previousVersion}/${result.previousVersionCode}`
  const after = `${result.version}/${result.versionCode}`
  process.stdout.write(
    result.changed
      ? `${APP_JSON}: ${before} -> ${after}\n`
      : `${APP_JSON}: already at ${after}, nothing to do\n`,
  )
}

const entryPoint = process.argv[1]
if (entryPoint && import.meta.url === pathToFileURL(path.resolve(entryPoint)).href) {
  try {
    main()
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    console.error(message.startsWith('NATIVE_APP_VERSION:') ? message : `NATIVE_APP_VERSION: ${message}`)
    process.exitCode = 1
  }
}

export { syncNativeAppVersion }
