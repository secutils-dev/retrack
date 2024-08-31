export class Diagnostics {
  public static errorMessage(err: unknown): string {
    if (typeof err === 'string') {
      return err;
    }

    if (err && typeof err === 'object') {
      return (err as { message?: string }).message ?? 'Unknown error';
    }

    return 'UNKNOWN';
  }
}
