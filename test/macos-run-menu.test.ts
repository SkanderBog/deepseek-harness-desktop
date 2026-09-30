import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const builderSource = readFileSync(new URL('../src-tauri/src/desktop/builder.rs', import.meta.url), 'utf8')
const navbarSource = readFileSync(new URL('../src/layout/components/navbar.tsx', import.meta.url), 'utf8')
const i18nSource = readFileSync(new URL('../src-tauri/src/config/i18n.rs', import.meta.url), 'utf8')

function submenuItems(id: string) {
  const match = builderSource.match(new RegExp(`"${id}",\\s*crate::config::i18n::t\\("[^"]+"\\),\\s*true,\\s*&\\[([^\\]]+)\\]`))
  expect(match, `${id} must be installed with native menu items`).not.toBeNull()
  return match![1].match(/&\w+/g)
}

describe('macOS native run menu', () => {
  it('installs File, Run, Help in order while retaining required macOS system and editing actions', () => {
    const nativeMenuSource = builderSource.slice(builderSource.indexOf('pub fn install_macos_menu('))
    const menu = nativeMenuSource.match(/let menu = Menu::with_items\(\s*app,\s*&\[([^\]]+)\]/)
    expect(menu).not.toBeNull()
    expect(menu![1].match(/&\w+/g)).toEqual([
      '&system_application_menu',
      '&file_menu',
      '&run_menu',
      '&edit_menu',
      '&help_menu',
    ])
    expect(builderSource).not.toContain('"desktop-application-menu"')
  })

  it('places Application, Profiles, Plugins, Core and fullscreen in the Run submenu', () => {
    expect(submenuItems('desktop-run-menu')).toEqual([
      '&config',
      '&profiles',
      '&plugins',
      '&harness',
      '&run_separator',
      '&fullscreen',
    ])
    expect(builderSource).toMatch(/"desktop-run-menu",\s*crate::config::i18n::t\("menu.run"\)/)
  })

  it.each([
    ['desktop-config', 'application', 'menu.application', '应用', 'Application'],
    ['desktop-profiles', 'profiles', 'menu.profiles', '档案', 'Profiles'],
    ['desktop-plugins', 'plugins', 'menu.plugins', '插件', 'Plugins'],
    ['desktop-harness', 'harness', 'menu.harness', '核心', 'Core'],
  ])('routes %s from its native item to the %s configuration tab', (action, tab, key, zh, en) => {
    expect(builderSource).toMatch(new RegExp(`MenuItem::with_id\\(\\s*app,\\s*"${action}",\\s*crate::config::i18n::t\\("${key}"\\)`))
    const eventHandler = builderSource.slice(builderSource.indexOf('.on_menu_event(|app, event|'))
    expect(eventHandler).toMatch(new RegExp(`"${action}"[\\s\\S]*?app.emit\\("macos-menu-action", event.id\\(\\).as_ref\\(\\)\\)`))
    expect(navbarSource).toMatch(new RegExp(`case '${action}':\\s*handleOpenConfig\\('${tab}'\\)`))
    expect(i18nSource).toContain(`"${key}" => ("${zh}", "${en}")`)
  })

  it('provides the native Run title in both languages', () => {
    expect(i18nSource).toContain('"menu.run" => ("运行", "Run")')
  })

  it('keeps fullscreen as a native action with state-dependent labels', () => {
    expect(builderSource).toContain('PredefinedMenuItem::fullscreen(app, Some(&fullscreen_label))?')
    expect(builderSource).toMatch(/let fullscreen_label = crate::config::i18n::t\(if is_fullscreen \{\s*"menu.exit_fullscreen"\s*\} else \{\s*"menu.enter_fullscreen"/)
    expect(builderSource).toContain('Some(fullscreen)')
    expect(builderSource).toContain('sync_macos_fullscreen_menu(window)')
  })
})
