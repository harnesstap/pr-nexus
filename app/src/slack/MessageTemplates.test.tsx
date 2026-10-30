import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, MessageTemplate, TemplateKind } from "../api";
import { MessageTemplates } from "./MessageTemplates";

const DEFAULTS: Record<TemplateKind, string> = {
  single_request: "Hello team\nCould someone please review {title}?{who}\n{url}\nThanks!",
  batch_request: "Hello team\nCould someone please review these pull requests?{who}\n{items}\nThanks!",
  single_remind: "Hello, I'm still looking for a review on {title} when you have a moment :pray:\n{url}\nThanks!",
  batch_remind: "Hello, I'm still looking for a review on these pull requests when you have a moment :pray:\n{items}\nThanks!",
  item: "- {title}\n{url}",
};

/** Stands in for the backend: unknown placeholders and a missing {url} are refused. */
function fakeApi(saved: [TemplateKind, string | null][]): Partial<Api> {
  const check = (kind: TemplateKind, text: string) => {
    const required = kind.startsWith("batch_") ? "{items}" : "{url}";
    const unknown = /\{(?!title\}|url\}|who\}|requester\}|count\}|items\})[^}]*\}/.exec(text);
    if (unknown) throw new Error(`unknown placeholder \`${unknown[0]}\``);
    if (!text.includes(required)) throw new Error(`the template must contain \`${required}\``);
  };
  return {
    messageTemplates: () =>
      Promise.resolve(Object.entries(DEFAULTS).map(([kind, text]) => ({ kind, text, default_text: text }) as MessageTemplate)),
    previewMessageTemplate: (kind, text) => {
      try {
        check(kind, text);
      } catch (err) {
        return Promise.reject(err);
      }
      return Promise.resolve(text.replace("{title}", "`fix: handle user_id in 'parser'`").replace("{url}", "https://github.com/acme/widgets/pull/42").replace("{who}", ""));
    },
    saveMessageTemplate: (kind, text) => {
      if (text !== null) check(kind, text);
      saved.push([kind, text]);
      return Promise.resolve();
    },
    openExternal: () => Promise.resolve(),
  };
}

test("edit shows a live preview, saves, and resets to the default", async () => {
  const saved: [TemplateKind, string | null][] = [];
  render(<MessageTemplates api={fakeApi(saved) as Api} />);
  const editor = await screen.findByRole("textbox", { name: "Single request" });
  expect(editor).toHaveValue(DEFAULTS.single_request);
  await waitFor(() => expect(screen.getByLabelText("Single request preview")).toHaveTextContent("fix: handle user_id in 'parser'"));
  expect(screen.getByRole("button", { name: "Save Single request" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Reset Single request to default" })).toBeDisabled();

  fireEvent.change(editor, { target: { value: "PTAL {title}\n{url}" } });
  await waitFor(() => expect(screen.getByLabelText("Single request preview")).toHaveTextContent("PTAL"));
  fireEvent.click(screen.getByRole("button", { name: "Save Single request" }));
  await waitFor(() => expect(saved).toEqual([["single_request", "PTAL {title}\n{url}"]]));

  fireEvent.click(screen.getByRole("button", { name: "Reset Single request to default" }));
  await waitFor(() => expect(saved[1]).toEqual(["single_request", null]));
  expect(editor).toHaveValue(DEFAULTS.single_request);
});

test("an unknown placeholder shows the error inline and the save is refused", async () => {
  const saved: [TemplateKind, string | null][] = [];
  render(<MessageTemplates api={fakeApi(saved) as Api} />);
  const editor = await screen.findByRole("textbox", { name: "Single reminder" });
  fireEvent.change(editor, { target: { value: "Ping {branch}\n{url}" } });
  expect(await screen.findByRole("alert")).toHaveTextContent("unknown placeholder `{branch}`");
  expect(editor).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save Single reminder" })).toBeDisabled();
  expect(screen.queryByLabelText("Single reminder preview")).not.toBeInTheDocument();
  expect(saved).toEqual([]);

  fireEvent.change(editor, { target: { value: "Ping" } });
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("must contain `{url}`"));
});
