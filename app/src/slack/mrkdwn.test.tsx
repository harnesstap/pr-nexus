import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { Mrkdwn, parseInline, parseMrkdwn } from "./mrkdwn";

test("links show their text; channels, people and groups show their names", () => {
  expect(parseInline("see <https://acme.example/pr/1|the PR> or <https://acme.example>")).toEqual([
    { kind: "text", text: "see " },
    { kind: "link", url: "https://acme.example/pr/1", text: "the PR" },
    { kind: "text", text: " or " },
    { kind: "link", url: "https://acme.example", text: "https://acme.example" },
  ]);
  expect(parseInline("<#C0123456789|eng-web> <@U0ACME0001|ada> <!subteam^S0ACME01|@web-team> <!here>").filter((span) => span.kind === "mention")).toEqual([
    { kind: "mention", text: "#eng-web" },
    { kind: "mention", text: "@ada" },
    { kind: "mention", text: "@web-team" },
    { kind: "mention", text: "@here" },
  ]);
});

test("bare URLs and markdown links become links; trailing punctuation stays text", () => {
  expect(parseInline("Open https://github.com/acme/web/pull/42.")).toEqual([
    { kind: "text", text: "Open " },
    { kind: "link", url: "https://github.com/acme/web/pull/42", text: "https://github.com/acme/web/pull/42" },
    { kind: "text", text: "." },
  ]);
  expect(parseInline("[Fix cache](https://acme.example/1)")).toEqual([{ kind: "link", url: "https://acme.example/1", text: "Fix cache" }]);
});

test("bold, italic, strike, code and emoji; snake_case and unknown emoji stay as typed", () => {
  const kinds = parseInline("*bold* _it_ ~gone~ `code` :rocket: :not_an_emoji: snake_case_name a*b").map((span) => span.kind);
  expect(kinds).toEqual(["bold", "text", "italic", "text", "strike", "text", "code", "text", "emoji", "text"]);
  const tail = parseInline(":not_an_emoji: snake_case_name a*b");
  expect(tail.map((span) => ("text" in span ? span.text : "")).join("")).toBe(":not_an_emoji: snake_case_name a*b");
  expect(parseInline("&lt;b&gt; &amp;")).toEqual([{ kind: "text", text: "<b> &" }]);
});

test("lines and quotes; nothing is rendered as HTML", () => {
  expect(parseMrkdwn("one\n&gt; quoted\n").map((line) => line.quote)).toEqual([false, true]);
  render(<Mrkdwn label="m" text={"*Hi* <img src=x onerror=alert(1)>\n> note"} onOpen={() => undefined} />);
  const box = screen.getByLabelText("m");
  expect(box.querySelector("img")).toBeNull();
  expect(box.querySelector("blockquote")?.textContent).toBe("note");
  expect(box.querySelector("strong")?.textContent).toBe("Hi");
});
