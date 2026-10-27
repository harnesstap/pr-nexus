import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";
import { guardTextFields } from "./textInput";

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

// Spreading first and overriding after would turn a behavior back on.
test("no text field overrides plainText", () => {
  const overrides = sources(join(__dirname))
    .flatMap((path) => fields(readFileSync(path, "utf8")).map((tag) => ({ path, tag })))
    .filter(({ tag }) => {
      const prose = tag.startsWith("<textarea") && tag.includes("data-prose");
      return /\bauto(Correct|Capitalize|Complete)=/.test(tag) || (!prose && /\bspellCheck\b/.test(tag));
    })
    .map(({ path, tag }) => `${path}: ${tag.split("\n")[0]}`);
  expect(overrides).toEqual([]);
});

test("guardTextFields covers existing, added and changed fields", async () => {
  document.body.innerHTML = '<input aria-label="existing"><input type="checkbox" aria-label="box">';
  const stop = guardTextFields(document);
  const attrs = (label: string) => {
    const field = document.querySelector(`[aria-label="${label}"]`)!;
    return ["autocorrect", "autocapitalize", "autocomplete", "spellcheck"].map((name) => field.getAttribute(name));
  };
  expect(attrs("existing")).toEqual(["off", "off", "off", "false"]);
  expect(attrs("box")).toEqual([null, null, null, null]);

  const added = document.createElement("div");
  added.innerHTML = '<textarea aria-label="added"></textarea><textarea data-prose aria-label="message"></textarea>';
  document.body.append(added);
  await Promise.resolve();
  expect(attrs("added")).toEqual(["off", "off", "off", "false"]);
  expect(attrs("message")).toEqual(["off", "off", "off", null]);

  document.querySelector('[aria-label="existing"]')!.setAttribute("autocorrect", "on");
  await Promise.resolve();
  expect(attrs("existing")[0]).toBe("off");
  stop();
  document.body.innerHTML = "";
});
