// SPDX-License-Identifier: MIT

import {
  ScriptKind,
  ScriptTarget,
  createSourceFile,
  forEachChild,
  isArrayLiteralExpression,
  isBindingElement,
  isCallExpression,
  isElementAccessExpression,
  isForOfStatement,
  isFunctionDeclaration,
  isFunctionExpression,
  isIdentifier,
  isMethodDeclaration,
  isNoSubstitutionTemplateLiteral,
  isPropertyAccessExpression,
  isSpreadElement,
  isStringLiteral,
  type BindingElement,
  type Expression,
  type Node,
  type SourceFile,
} from "typescript";

import { ALLOWED_REQUEST_HEADERS } from "../src/privacy.js";

const FORBIDDEN_HEADER_NAMES = [
  "cf-connecting-ip",
  "x-forwarded-for",
  "user-agent",
  "x-real-ip",
  "cf-ipcountry",
  "clientip",
];

const SENSITIVE_IDS = new Set(["token", "request", "headers"]);

export function lintTypeScript(fileName: string, sourceText: string): string[] {
  const sf = createSourceFile(fileName, sourceText, ScriptTarget.Latest, true, ScriptKind.TS);
  const errors: string[] = [];
  const visit = (node: Node): void => {
    checkForbiddenHeaderLiteral(node, sf, errors);
    checkCfAccess(node, sf, errors);
    checkHeadersProperty(node, sf, errors);
    checkHeadersGet(node, sf, errors);
    checkHeadersIteration(node, sf, errors);
    checkConsole(node, sf, errors);
    forEachChild(node, visit);
  };
  visit(sf);
  if (fileName.endsWith("privacy.ts") || fileName.replace(/\\/g, "/").endsWith("/privacy.ts")) {
    errors.push(...lintAllowedSet(sourceText));
  }
  return errors;
}

function checkForbiddenHeaderLiteral(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isStringLiteral(node) && !isNoSubstitutionTemplateLiteral(node)) {
    return;
  }
  const value = node.text.toLowerCase();
  for (const name of FORBIDDEN_HEADER_NAMES) {
    if (value === name || value.includes(name)) {
      errors.push(`${loc(sf, node)}: forbidden header name ${name}`);
    }
  }
}

function checkCfAccess(node: Node, sf: SourceFile, errors: string[]): void {
  if (isPropertyAccessExpression(node) && node.name.text.toLowerCase() === "cf") {
    errors.push(`${loc(sf, node)}: request.cf access is forbidden`);
  }
  if (isElementAccessExpression(node) && isText(node.argumentExpression, "cf")) {
    errors.push(`${loc(sf, node)}: request["cf"] access is forbidden`);
  }
  if (isBindingElement(node)) {
    const name = bindingName(node);
    if (name === "cf") {
      errors.push(`${loc(sf, node)}: destructuring cf is forbidden`);
    }
  }
}

function checkHeadersProperty(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isPropertyAccessExpression(node) || node.name.text !== "headers") {
    return;
  }
  if (isInsideReadAllowedHeader(node)) {
    return;
  }
  const parent = node.parent;
  if (
    isCallExpression(parent) &&
    isIdentifier(parent.expression) &&
    parent.expression.text === "readAllowedHeader" &&
    parent.arguments[0] === node
  ) {
    return;
  }
  errors.push(`${loc(sf, node)}: request.headers is only allowed as the argument to readAllowedHeader`);
}

function checkHeadersGet(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isCallExpression(node) || !isPropertyAccessExpression(node.expression)) {
    return;
  }
  if (node.expression.name.text !== "get") {
    return;
  }
  if (!isHeadersReceiver(node.expression.expression)) {
    return;
  }
  if (isInsideReadAllowedHeader(node)) {
    return;
  }
  const arg = node.arguments[0];
  if (arg === undefined || !isStringLiteral(arg)) {
    errors.push(`${loc(sf, node)}: headers.get must use an allowlisted string literal`);
    return;
  }
  if (!isAllowedHeader(arg.text)) {
    errors.push(`${loc(sf, node)}: headers.get(${arg.text}) is not allowlisted`);
  }
}

function checkHeadersIteration(node: Node, sf: SourceFile, errors: string[]): void {
  if (isSpreadElement(node) && isHeadersReceiver(node.expression)) {
    errors.push(`${loc(sf, node)}: iterating headers is forbidden`);
  }
  if (isForOfStatement(node) && isHeadersReceiver(node.expression) && !isInsideReadAllowedHeader(node)) {
    errors.push(`${loc(sf, node)}: iterating headers is forbidden`);
  }
  if (isCallExpression(node) && isPropertyAccessExpression(node.expression)) {
    const method = node.expression.name.text;
    if (
      (method === "forEach" || method === "entries" || method === "keys" || method === "values") &&
      isHeadersReceiver(node.expression.expression)
    ) {
      errors.push(`${loc(sf, node)}: iterating headers is forbidden`);
    }
  }
}

function checkConsole(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isCallExpression(node) || !isPropertyAccessExpression(node.expression)) {
    return;
  }
  if (!isIdentifier(node.expression.expression) || node.expression.expression.text !== "console") {
    return;
  }
  for (const arg of node.arguments) {
    if (mentionsSensitive(arg)) {
      errors.push(
        `${loc(sf, node)}: console.${node.expression.name.text} must not receive request, headers, or token`,
      );
    }
  }
}

function mentionsSensitive(node: Node): boolean {
  if (isIdentifier(node) && SENSITIVE_IDS.has(node.text)) {
    return true;
  }
  if (isPropertyAccessExpression(node) || isElementAccessExpression(node)) {
    return mentionsSensitive(node.expression);
  }
  if (isSpreadElement(node)) {
    return mentionsSensitive(node.expression);
  }
  if (isArrayLiteralExpression(node) || isCallExpression(node)) {
    const args = isCallExpression(node) ? node.arguments : node.elements;
    return args.some((child) => mentionsSensitive(child));
  }
  let found = false;
  forEachChild(node, (child) => {
    if (mentionsSensitive(child)) {
      found = true;
    }
  });
  return found;
}

function lintAllowedSet(sourceText: string): string[] {
  const expected = [...ALLOWED_REQUEST_HEADERS].sort();
  const match = sourceText.match(/ALLOWED_REQUEST_HEADERS\s*=\s*\[([\s\S]*?)\]\s*as const/);
  if (match === null || match[1] === undefined) {
    return ["privacy.ts must export ALLOWED_REQUEST_HEADERS as a const array"];
  }
  const names = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();
  if (names.length !== expected.length || names.some((name, i) => name !== expected[i])) {
    return [`privacy.ts ALLOWED_REQUEST_HEADERS must be exactly ${expected.join(", ")}`];
  }
  for (const name of names) {
    if (name === "user-agent" || FORBIDDEN_HEADER_NAMES.includes(name ?? "")) {
      return [`privacy.ts must not allow ${name}`];
    }
  }
  return [];
}

function isAllowedHeader(name: string): boolean {
  return (ALLOWED_REQUEST_HEADERS as readonly string[]).includes(name.toLowerCase());
}

function isHeadersReceiver(node: Node): boolean {
  if (isIdentifier(node) && node.text === "headers") {
    return true;
  }
  if (isPropertyAccessExpression(node) && node.name.text === "headers") {
    return true;
  }
  return false;
}

function isInsideReadAllowedHeader(node: Node): boolean {
  let current: Node | undefined = node.parent;
  while (current !== undefined) {
    if (isFunctionDeclaration(current) || isFunctionExpression(current) || isMethodDeclaration(current)) {
      const name = current.name;
      if (name !== undefined && isIdentifier(name) && name.text === "readAllowedHeader") {
        return true;
      }
    }
    current = current.parent;
  }
  return false;
}

function isText(node: Expression | undefined, value: string): boolean {
  return node !== undefined && isStringLiteral(node) && node.text.toLowerCase() === value;
}

function bindingName(node: BindingElement): string | undefined {
  if (isIdentifier(node.name)) {
    return node.name.text;
  }
  if (node.propertyName !== undefined && isIdentifier(node.propertyName)) {
    return node.propertyName.text;
  }
  return undefined;
}

function loc(sf: SourceFile, node: Node): string {
  const { line, character } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
  return `${sf.fileName}:${line + 1}:${character + 1}`;
}
