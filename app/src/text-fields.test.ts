import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";

const NON_TEXT = /type="(checkbox|radio|color|range|file|hidden)"/;

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      return sources(path);
    }
    return entry.name.endsWith(".tsx") && !entry.name.endsWith(".test.tsx") ? [path] : [];
  });
}

/** The JSX of each `<input …>` / `<textarea …>` opening tag, braces balanced. */
function fields(source: string): string[] {
  const tags: string[] = [];
  for (const match of source.matchAll(/<(input|textarea)(?=[\s/])/g)) {
    let depth = 0;
    let end = match.index;
    for (; end < source.length; end++) {
      const char = source[end];
      if (char === "{") depth++;
      else if (char === "}") depth--;
      else if (char === ">" && depth === 0) break;
    }
    tags.push(source.slice(match.index, end));
  }
  return tags;
}

// §8.2 "Text fields": no spellcheck, autocorrect, autocapitalize or autofill.
test("every text field spreads plainText", () => {
  const missing = sources(join(__dirname))
    .flatMap((path) => fields(readFileSync(path, "utf8")).map((tag) => ({ path, tag })))
    .filter(({ tag }) => !NON_TEXT.test(tag) && !tag.includes("{...plainText}"))
    .map(({ path, tag }) => `${path}: ${tag.split("\n")[0]}`);
  expect(missing).toEqual([]);
});
