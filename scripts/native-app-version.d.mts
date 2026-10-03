/**
 * `scripts/native-app-version.mjs` 的类型声明。
 *
 * 该脚本由 `bump.config.ts` 的 `execute` 调用，是给 `bumpp` 直接跑的纯 ESM，
 * 本身不带类型；这里声明它导出的函数，供单测与 typecheck 解析。
 */

export interface SyncNativeAppVersionResult {
  /** `app.json` 的绝对路径。 */
  file: string
  /** `false` 表示 `expo.version` 已与桌面端版本一致，未写入文件。 */
  changed: boolean
  /** 桌面端 `package.json` 的版本，也是写入 `expo.version` 的目标值。 */
  version: string
  /** 写入后的 `expo.android.versionCode`（未写入时为原值）。 */
  versionCode: number
  previousVersion: string
  previousVersionCode: number
}

/**
 * 让 `app.json` 的 `expo.version` 跟随桌面端版本；`version` 变化时 `expo.android.versionCode` +1。
 *
 * 幂等：`expo.version` 已等于桌面端版本时直接返回且不写文件。字段缺失或类型不对时抛错，
 * 不会静默改写 `app.json`。
 */
export function syncNativeAppVersion(options?: { repo?: string }): SyncNativeAppVersionResult
