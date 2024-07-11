export type ApiResult<R> = { type: 'success'; data: R } | { type: 'client-error'; error: string };
