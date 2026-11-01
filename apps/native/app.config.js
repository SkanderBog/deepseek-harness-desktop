import { readFileSync } from 'node:fs'
import configPlugins from 'expo/config-plugins.js'

const { AndroidConfig, withAndroidManifest, withMainActivity } = configPlugins
const notices = new URL('../../THIRD_PARTY_NOTICES.md', import.meta.url)
const mainActivityClass = 'class MainActivity : ReactActivity() {'
const superOnCreate = /^([ \t]*)super\.onCreate\(/m
const orientationMembers = '\n  override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {\n    super.onConfigurationChanged(newConfig)\n    applyScreenOrientation(newConfig)\n  }\n\n  private fun applyScreenOrientation(config: android.content.res.Configuration) {\n    requestedOrientation = if (config.smallestScreenWidthDp < 600) {\n      android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT\n    } else {\n      android.content.pm.ActivityInfo.SCREEN_ORIENTATION_FULL_USER\n    }\n  }\n'

export default function appConfig({ config }) {
  const withNotices = { ...config, extra: { ...config.extra, thirdPartyNotices: readFileSync(notices, 'utf8') } }
  const withManifest = withAndroidManifest(withNotices, (value) => {
    AndroidConfig.Manifest.getMainActivityOrThrow(value.modResults).$['android:screenOrientation'] = 'fullUser'
    return value
  })
  return withMainActivity(withManifest, (value) => {
    const { contents } = value.modResults
    if (contents.includes('applyScreenOrientation('))
      return value
    if (!contents.includes(mainActivityClass) || !superOnCreate.test(contents))
      throw new Error('MainActivity.kt no longer matches the Expo template that the tablet orientation patch expects')
    value.modResults.contents = contents.replace(superOnCreate, '$1applyScreenOrientation(resources.configuration)\n$1super.onCreate(').replace(mainActivityClass, () => `${mainActivityClass}\n${orientationMembers}`)
    return value
  })
}
