// SPDX-License-Identifier: MIT

import {
  ScriptKind,
  ScriptTarget,
  SyntaxKind,
  createSourceFile,
  forEachChild,
  isAsExpression,
  isBinaryExpression,
  isBindingElement,
  isCallExpression,
  isElementAccessExpression,
  isForOfStatement,
  isFunctionDeclaration,
  isIdentifier,
  isMethodDeclaration,
  isNewExpression,
  isNoSubstitutionTemplateLiteral,
  isParameter,
  isParenthesizedExpression,
  isPropertyAccessExpression,
  isPropertyAssignment,
  isPropertySignature,
  isShorthandPropertyAssignment,
  isSpreadElement,
  isStringLiteral,
  isTypeAssertionExpression,
  isVariableDeclaration,
  type BindingElement,
  type Expression,
  type Identifier,
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

const FORBIDDEN_GLOBALS = new Set(["console", "eval", "Function", "globalThis", "Reflect", "self"]);
const SECRET_BINDINGS = new Set(["token", "secret", "turnstileToken"]);
const COMPUTED_ROOTS = new Set(["request", "headers", "env"]);
const OBJECT_REFLECT = new Set([
  "entries",
  "fromEntries",
  "getOwnPropertyDescriptor",
  "getOwnPropertyDescriptors",
  "assign",
  "keys",
  "values",
]);
const ALLOWED_SECRET_CALLEES = new Set(["verify", "encodeURIComponent"]);

export function lintTypeScript(fileName: string, sourceText: string): string[] {
  const sf = createSourceFile(fileName, sourceText, ScriptTarget.Latest, true, ScriptKind.TS);
  const aliases = collectAliases(sf);
  const errors: string[] = [];
  const visit = (node: Node): void => {
    checkForbiddenHeaderLiteral(node, sf, errors);
    checkCfAccess(node, sf, errors);
    checkHeadersProperty(node, sf, errors);
    checkHeadersGet(node, sf, errors);
    checkHeadersIteration(node, sf, errors, aliases.roots);
    checkReadAllowedHeader(node, sf, errors);
    checkForbiddenGlobals(node, sf, fileName, errors);
    checkSecretBindings(node, sf, errors, aliases.secrets);
    checkComputedAccess(node, sf, errors, aliases.roots);
    checkObjectReflection(node, sf, errors, aliases.roots);
    forEachChild(node, visit);
  };
  visit(sf);
  if (normalizePath(fileName).endsWith("privacy.ts")) {
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
    if (name === "headers" || (node.propertyName !== undefined && isIdentifier(node.propertyName) && node.propertyName.text === "headers")) {
      if (!isPrivacySourceFile(sf)) {
        errors.push(`${loc(sf, node)}: destructuring headers is forbidden`);
      }
    }
  }
}

function checkHeadersProperty(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isPropertyAccessExpression(node) || node.name.text !== "headers") {
    return;
  }
  if (isPrivacySourceFile(sf)) {
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
  if (isPrivacySourceFile(sf)) {
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

function checkHeadersIteration(
  node: Node,
  sf: SourceFile,
  errors: string[],
  roots: ReadonlyMap<string, string>,
): void {
  if (isSpreadElement(node) && isSensitiveExpression(node.expression, roots)) {
    errors.push(`${loc(sf, node)}: spreading request, headers, or env is forbidden`);
  }
  if (isForOfStatement(node) && isSensitiveExpression(node.expression, roots) && !isPrivacySourceFile(sf)) {
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

function checkReadAllowedHeader(node: Node, sf: SourceFile, errors: string[]): void {
  if (!isCallExpression(node) || !isIdentifier(node.expression) || node.expression.text !== "readAllowedHeader") {
    return;
  }
  const nameArg = node.arguments[1];
  if (nameArg === undefined || !isStringLiteral(nameArg)) {
    errors.push(`${loc(sf, node)}: readAllowedHeader name must be an allowlisted string literal`);
    return;
  }
  if (!isAllowedHeader(nameArg.text)) {
    errors.push(`${loc(sf, node)}: readAllowedHeader(${nameArg.text}) is not allowlisted`);
  }
}

function checkForbiddenGlobals(node: Node, sf: SourceFile, fileName: string, errors: string[]): void {
  if (!isIdentifier(node) || !FORBIDDEN_GLOBALS.has(node.text)) {
    return;
  }
  if (isPropertyNameOnly(node)) {
    return;
  }
  if (node.text === "console" && isLogModule(fileName)) {
    return;
  }
  errors.push(`${loc(sf, node)}: identifier ${node.text} is forbidden outside the allowlisted log module`);
}

function checkSecretBindings(
  node: Node,
  sf: SourceFile,
  errors: string[],
  secrets: ReadonlySet<string>,
): void {
  if (!isIdentifier(node) || (!SECRET_BINDINGS.has(node.text) && !secrets.has(node.text))) {
    return;
  }
  if (isDeclarationName(node) || isPropertyNameOnly(node)) {
    return;
  }
  const parent = node.parent;
  if (isPropertyAccessExpression(parent) && parent.expression === node) {
    if (parent.name.text === "length") {
      return;
    }
    errors.push(
      `${loc(sf, node)}: identifier ${node.text} is only allowed in the Turnstile client call`,
    );
    return;
  }
  if (isBinaryExpression(parent) && parent.operatorToken.kind !== SyntaxKind.PlusToken) {
    return;
  }
  if (isShorthandPropertyAssignment(parent)) {
    errors.push(`${loc(sf, node)}: identifier ${node.text} cannot be placed in an object literal`);
    return;
  }
  if (isPropertyAssignment(parent) && parent.initializer === node) {
    errors.push(`${loc(sf, node)}: identifier ${node.text} cannot be placed in an object literal`);
    return;
  }
  if ((isCallExpression(parent) || isNewExpression(parent)) && (parent.arguments ?? []).some((arg) => arg === node)) {
    if (isAllowedSecretCallee(parent.expression)) {
      return;
    }
    errors.push(
      `${loc(sf, node)}: identifier ${node.text} is only allowed in the Turnstile client call`,
    );
    return;
  }
  errors.push(
    `${loc(sf, node)}: identifier ${node.text} is only allowed in the Turnstile client call`,
  );
}

function checkComputedAccess(
  node: Node,
  sf: SourceFile,
  errors: string[],
  roots: ReadonlyMap<string, string>,
): void {
  if (!isElementAccessExpression(node)) {
    return;
  }
  const root = rootIdentifier(unwrapExpr(node.expression), roots);
  if (root === undefined || !COMPUTED_ROOTS.has(root)) {
    return;
  }
  if (!isStringLiteral(node.argumentExpression) && !isNoSubstitutionTemplateLiteral(node.argumentExpression)) {
    errors.push(`${loc(sf, node)}: computed or concatenated member access on ${root} is forbidden`);
  }
}

function checkObjectReflection(
  node: Node,
  sf: SourceFile,
  errors: string[],
  roots: ReadonlyMap<string, string>,
): void {
  if (!isCallExpression(node) || !isPropertyAccessExpression(node.expression)) {
    return;
  }
  if (!isIdentifier(node.expression.expression) || node.expression.expression.text !== "Object") {
    return;
  }
  const method = node.expression.name.text;
  if (!OBJECT_REFLECT.has(method)) {
    return;
  }
  for (const arg of node.arguments) {
    if (isSensitiveExpression(arg, roots)) {
      errors.push(`${loc(sf, node)}: Object.${method} on request, headers, or env is forbidden`);
    }
  }
}

function collectAliases(sf: SourceFile): { roots: Map<string, string>; secrets: Set<string> } {
  const roots = new Map<string, string>();
  const secrets = new Set<string>();
  const visit = (node: Node): void => {
    if (isVariableDeclaration(node) && isIdentifier(node.name) && node.initializer !== undefined) {
      const init = unwrapExpr(node.initializer);
      if (isIdentifier(init)) {
        const root = roots.get(init.text) ?? (COMPUTED_ROOTS.has(init.text) ? init.text : undefined);
        if (root !== undefined) {
          roots.set(node.name.text, root);
        }
        if (SECRET_BINDINGS.has(init.text) || secrets.has(init.text)) {
          secrets.add(node.name.text);
        }
      }
    }
    forEachChild(node, visit);
  };
  visit(sf);
  return { roots, secrets };
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
  const inner = unwrapExpr(node as Expression);
  if (isIdentifier(inner) && inner.text === "headers") {
    return true;
  }
  if (isPropertyAccessExpression(inner) && inner.name.text === "headers") {
    return true;
  }
  if (isElementAccessExpression(inner) && isText(inner.argumentExpression, "headers")) {
    return true;
  }
  return false;
}

function isSensitiveExpression(node: Node, roots: ReadonlyMap<string, string>): boolean {
  if (!isExpressionLike(node)) {
    return false;
  }
  const root = rootIdentifier(unwrapExpr(node), roots);
  return root !== undefined && COMPUTED_ROOTS.has(root);
}

function isExpressionLike(node: Node): node is Expression {
  return (node as Expression).kind !== undefined && "parent" in node;
}

function unwrapExpr(node: Expression): Expression {
  let current = node;
  for (;;) {
    if (isParenthesizedExpression(current)) {
      current = current.expression;
      continue;
    }
    if (isAsExpression(current) || isTypeAssertionExpression(current)) {
      current = current.expression;
      continue;
    }
    return current;
  }
}

function isDeclarationName(node: Identifier): boolean {
  const parent = node.parent;
  if (isParameter(parent) && parent.name === node) {
    return true;
  }
  if (isVariableDeclaration(parent) && parent.name === node) {
    return true;
  }
  if (isFunctionDeclaration(parent) && parent.name === node) {
    return true;
  }
  if (isMethodDeclaration(parent) && parent.name === node) {
    return true;
  }
  if (isBindingElement(parent) && isIdentifier(parent.name) && parent.name === node) {
    return true;
  }
  return false;
}

function isAllowedSecretCallee(expr: Expression): boolean {
  const inner = unwrapExpr(expr);
  if (isIdentifier(inner) && ALLOWED_SECRET_CALLEES.has(inner.text)) {
    return true;
  }
  return isPropertyAccessExpression(inner) && ALLOWED_SECRET_CALLEES.has(inner.name.text);
}

function isPrivacySourceFile(sf: SourceFile): boolean {
  const name = normalizePath(sf.fileName);
  return name.endsWith("/privacy.ts") || name.endsWith("src/privacy.ts") || name === "privacy.ts";
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

function isPropertyNameOnly(node: Identifier): boolean {
  const parent = node.parent;
  if (isPropertyAssignment(parent) && parent.name === node) {
    return true;
  }
  if (isPropertySignature(parent) && parent.name === node) {
    return true;
  }
  if (isPropertyAccessExpression(parent) && parent.name === node) {
    return true;
  }
  if (parent.kind === SyntaxKind.MethodSignature) {
    const named = parent as { name?: Node };
    return named.name === node;
  }
  return false;
}

function rootIdentifier(node: Expression, roots: ReadonlyMap<string, string>): string | undefined {
  const inner = unwrapExpr(node);
  if (isIdentifier(inner)) {
    return roots.get(inner.text) ?? inner.text;
  }
  if (isPropertyAccessExpression(inner)) {
    return rootIdentifier(inner.expression, roots);
  }
  if (isElementAccessExpression(inner)) {
    return rootIdentifier(inner.expression, roots);
  }
  return undefined;
}

function isLogModule(fileName: string): boolean {
  return normalizePath(fileName).endsWith("/log.ts") || normalizePath(fileName).endsWith("src/log.ts") || normalizePath(fileName) === "log.ts";
}

function normalizePath(fileName: string): string {
  return fileName.replace(/\\/g, "/");
}

function loc(sf: SourceFile, node: Node): string {
  const { line, character } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
  return `${sf.fileName}:${line + 1}:${character + 1}`;
}
