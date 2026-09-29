import type { TaskInput } from '../types'
import { scheduler } from '../service/scheduler'
import { task } from '../service/task'
import { nullableText, scheduleParameters, textBlock } from '../utils/tool'

const TEXT_FIELDS = ['name', 'prompt', 'workspaceId', 'permission', 'provider', 'model', 'reasoningEffort'] as const

const outputSchema = {
  type: 'object',
  additionalProperties: true,
  properties: {
    ok: { type: 'boolean' },
    taskId: { type: 'string' },
    nextRunAt: nullableText,
    ran: { type: 'boolean' },
    error: { type: 'string' },
  },
  required: ['ok'],
}

export function updateTaskTool(): any {
  return {
    name: 'scheduler_update',
    description:
      'Update a scheduled task by id: rename it, rewrite its instruction, replace its schedule, '
      + 'pause or resume it (enabled), or change the workspace/permission/model used by its runs. '
      + 'Omitted fields keep their current value. Set run_now to also trigger an immediate manual run.',
    parameters: {
      type: 'object',
      properties: {
        task_id: { type: 'string', description: 'Task id.' },
        name: { type: 'string', description: 'New task name.' },
        prompt: { type: 'string', description: 'New task instruction.' },
        schedule: scheduleParameters,
        enabled: { type: 'boolean', description: 'true to resume, false to pause.' },
        workspaceId: { type: 'string', description: 'New target workspace id (cwd).' },
        permission: { type: 'string', enum: ['read-only', 'workspace-write', 'danger-full-access'], description: 'New permission boundary.' },
        provider: { type: 'string', description: 'New pinned model provider id (pair with model).' },
        model: { type: 'string', description: 'New pinned model id (pair with provider).' },
        reasoningEffort: { type: 'string', description: 'New pinned reasoning effort id.' },
        run_now: { type: 'boolean', description: 'Also trigger an immediate manual run after the update.' },
      },
      required: ['task_id'],
    },
    output: {
      schema: outputSchema,
      render: (_args: unknown, value: any) => value.ok
        ? textBlock(`✅ 定时任务已更新：${value.taskId}（下次运行 ${value.nextRunAt ?? '待计算'}）${value.ran === true ? '，已触发立即运行' : ''}`)
        : textBlock(`❌ ${value.error}`),
    },
    async execute(args: any) {
      const id = String(args.task_id ?? '')
      const result = await task.update(id, patchOf(args))
      if (!result.ok)
        return { ok: false, error: result.error }
      const updated = { ok: true, taskId: result.task.id, nextRunAt: result.task.nextRunAt ?? null }
      if (args.run_now !== true)
        return updated
      const run = await scheduler.trigger(id)
      if (!run.ok)
        return { ok: false, error: `更新已保存，但立即运行失败：${run.error}` }
      return { ...updated, ran: true }
    },
  }
}

// --- internal ---

function patchOf(args: any): Partial<TaskInput> {
  const patch: Record<string, unknown> = {}
  for (const field of TEXT_FIELDS) {
    if (args[field] !== undefined)
      patch[field] = String(args[field])
  }
  if (args.schedule !== undefined)
    patch.schedule = args.schedule
  if (args.enabled !== undefined)
    patch.enabled = args.enabled === true
  return patch as Partial<TaskInput>
}
