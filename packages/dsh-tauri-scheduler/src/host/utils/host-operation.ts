const HOST_OPERATION_TIMEOUT_MS = 30_000

export async function hostOperation<T>(operation: T): Promise<Awaited<T>> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('Scheduler host operation timed out')), HOST_OPERATION_TIMEOUT_MS)
      }),
    ])
  }
  finally {
    clearTimeout(timer)
  }
}
