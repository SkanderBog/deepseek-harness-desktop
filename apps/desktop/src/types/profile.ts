/** Rust 侧 service::profile::Profile 的序列化形态（camelCase） */
export interface Profile {
  /** 档案 id（$DSH_HOME/profiles/<id> 目录名） */
  id: string
  /** 展示名（manifest name 去 dsh-profile- 前缀，首字母大写） */
  name: string
  /** 是否 dsh 默认档案（web） */
  default: boolean
  /** 是否桌面端内置档案（desktop，可重置不可删除） */
  builtin: boolean
  /** 是否当前使用中的档案 */
  active: boolean
}
