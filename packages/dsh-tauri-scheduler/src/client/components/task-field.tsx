import type { ReactElement, ReactNode } from 'react'

export function TaskField({ label, as = 'div', children }: { label: ReactNode, as?: 'label' | 'div', children?: ReactNode }): ReactElement {
  const Tag = as
  return (
    <Tag className="flex flex-col gap-[2px] min-w-0 text-[13px]">
      <span className="inline-flex items-center gap-[10px] text-secondary text-[12px] font-medium leading-[18px]">{label}</span>
      {children}
    </Tag>
  )
}
