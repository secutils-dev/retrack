import jsonStableStringify from 'fast-json-stable-stringify';
import type { WorkerResultMessage } from './constants.js';

/**
 * Represents the result of the execution of a scenario.
 */
export class ExecutionResult {
  readonly value: string;
  readonly type: 'text' | 'html' | 'json';
  constructor(value: string, type: 'text' | 'html' | 'json') {
    this.value = value;
    this.type = type;
  }

  /**
   * Creates a new text result.
   * @param text
   */
  static text(text: string): ExecutionResult {
    return new ExecutionResult(text, 'text');
  }

  /**
   * Creates a new HTML result.
   * @param html
   */
  static html(html: string): ExecutionResult {
    return new ExecutionResult(html, 'html');
  }

  /**
   * Creates a new JSON result.
   * @param json
   */
  static json(json: unknown): ExecutionResult {
    return new ExecutionResult(jsonStableStringify(json), 'json');
  }

  toContent(): WorkerResultMessage['content'] {
    return { type: this.type, value: this.value };
  }
}
