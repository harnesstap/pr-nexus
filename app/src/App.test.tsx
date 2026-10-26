import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";

test("shows Hello", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "Hello" })).toBeInTheDocument();
});
