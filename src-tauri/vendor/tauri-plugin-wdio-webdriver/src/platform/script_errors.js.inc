try {
__WD_SCRIPT__
} catch (error) {
  ({ __wdio_script_error: String(error?.message ?? error) })
}
