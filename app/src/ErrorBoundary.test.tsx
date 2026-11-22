import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

function Boom(): never {
  throw new Error("The list could not be drawn.");
}

afterEach(() => {
  vi.restoreAllMocks();
});

test("a render throw shows the error, Reload, and Copy details instead of going blank", async () => {
  const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
  const reload = vi.fn();
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });

  render(
    <ErrorBoundary onReload={reload}>
      <Boom />
    </ErrorBoundary>,
  );

  const alert = screen.getByRole("alert");
  expect(alert).toHaveTextContent("Something went wrong");
  expect(alert).toHaveTextContent("The list could not be drawn.");
  expect(alert.className).toContain("bg-slate-950");
  expect(log).toHaveBeenCalledWith("PR Nexus render error", expect.any(Error), expect.any(String));

  fireEvent.click(screen.getByRole("button", { name: "Reload" }));
  expect(reload).toHaveBeenCalledTimes(1);

  fireEvent.click(screen.getByRole("button", { name: "Copy details" }));
  await waitFor(() => {
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining("The list could not be drawn."));
  });
  expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
});
