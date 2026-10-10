import type { MenuEntry } from 'dsh-tauri-ui/client'
import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactElement, ReactNode } from 'react'
import { createPortal } from 'react-dom'

export function PrimitiveButton({ variant: _variant, size: _size, icon, children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string, size?: string, icon?: ReactNode }): ReactElement {
  return (
    <button type="button" {...props}>
      {icon}
      {children}
    </button>
  )
}

export function PrimitiveInput(props: InputHTMLAttributes<HTMLInputElement>): ReactElement {
  return <input {...props} />
}

export function PrimitiveMenu({ open, anchor, items = [], onSelect, portal }: { open: boolean, anchor: ReactNode, items?: readonly MenuEntry[], onSelect?: (id: string) => void, portal?: boolean }): ReactElement {
  const menu = open
    ? (
        <div role="menu">
          {items.map(item => 'type' in item
            ? item.type === 'label' ? <span key={item.id}>{item.text}</span> : <hr key={item.id} />
            : <button type="button" role="menuitem" key={item.id} disabled={item.disabled} onClick={() => onSelect?.(item.id)}>{item.label}</button>)}
        </div>
      )
    : null
  return (
    <>
      {anchor}
      {portal && menu ? createPortal(menu, document.body) : menu}
    </>
  )
}

export async function schedulerClientUi() {
  const [button, card, checkbox, chip, icon, icons, select, text, textarea] = await Promise.all([
    import('../../../../dsh-tauri-ui/src/client/components/button'),
    import('../../../../dsh-tauri-ui/src/client/components/card'),
    import('../../../../dsh-tauri-ui/src/client/components/checkbox'),
    import('../../../../dsh-tauri-ui/src/client/components/chip'),
    import('../../../../dsh-tauri-ui/src/client/components/icon'),
    import('../../../../dsh-tauri-ui/src/client/components/icons'),
    import('../../../../dsh-tauri-ui/src/client/components/select'),
    import('../../../../dsh-tauri-ui/src/client/components/text'),
    import('../../../../dsh-tauri-ui/src/client/components/textarea'),
  ])
  return {
    Button: button.Button,
    Card: card.Card,
    Checkbox: checkbox.Checkbox,
    Chip: chip.Chip,
    Icon: icon.Icon,
    ChevronDown: icons.ChevronDown,
    Select: select.Select,
    Text: text.Text,
    Textarea: textarea.Textarea,
    Input: PrimitiveInput,
    Menu: PrimitiveMenu,
  }
}
