// SPDX-License-Identifier: MIT

/** The only Worker module allowed to touch `console`. */
export function logJson(value: unknown): void {
  console.log(JSON.stringify(value));
}
