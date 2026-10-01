export interface PlanSession {
  snapshotEvents?: () => readonly unknown[]
  append: (type: 'todo/write', data: { todos: readonly unknown[] }) => unknown
}

export type CreateUserMessage = (input: {
  content: readonly { type: 'text', text: string }[]
  source: unknown
}) => unknown
