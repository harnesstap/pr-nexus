import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync("src/index.css", "utf8");

describe("clickable cursor", () => {
  it("restores the pointer cursor that Tailwind preflight removes from buttons and links", () => {
    const rule = css.match(/([^{}]*)\{\s*cursor:\s*pointer;\s*\}/);
    expect(rule).not.toBeNull();
    const selectors = rule![1];
    for (const selector of ["button:not(:disabled)", '[role="button"]', '[role="option"]', "a[href]", 'input[type="checkbox"]']) {
      expect(selectors).toContain(selector);
    }
  });

  it("shows a not-allowed cursor on disabled controls", () => {
    expect(css).toMatch(/:disabled,\s*\[aria-disabled="true"\]\s*\{\s*cursor:\s*not-allowed;/);
  });
});
