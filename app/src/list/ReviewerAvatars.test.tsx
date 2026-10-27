import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { PendingReviewer } from "../api";
import { TOOLTIP_DELAY_MS } from "../Tooltip";
import { initials, MAX_AVATARS, ReviewerAvatars, reviewerLine, sizedAvatar, stackLayout } from "./ReviewerAvatars";

afterEach(() => vi.useRealTimers());

const user = (login: string, name: string | null = null, avatar: string | null = null): PendingReviewer => ({
  kind: "user",
  login,
  name,
  avatar_url: avatar,
});
const team = (login: string, name: string | null = null): PendingReviewer => ({ kind: "team", login, name, avatar_url: null });

test("up to three reviewers show as avatars with no chip; a fourth folds into +N", () => {
  const three = [user("ada"), user("bo"), team("acme/web")];
  expect(stackLayout(three)).toEqual({ shown: three, more: 0 });
  const five = [...three, user("cy"), team("acme/api")];
  expect(MAX_AVATARS).toBe(3);
  expect(stackLayout(five)).toEqual({ shown: three, more: 2 });

  const { rerender } = render(<ReviewerAvatars reviewers={three} />);
  expect(screen.getByTestId("reviewer-avatars").querySelectorAll("[data-reviewer]")).toHaveLength(3);
  expect(screen.queryByTestId("reviewer-more")).not.toBeInTheDocument();

  rerender(<ReviewerAvatars reviewers={five} />);
  const drawn = [...screen.getByTestId("reviewer-avatars").querySelectorAll("[data-reviewer]")].map((node) => node.getAttribute("data-reviewer"));
  expect(drawn).toEqual(["ada", "bo", "acme/web"]);
  expect(screen.getByTestId("reviewer-more")).toHaveTextContent("+2");
});

test("no pending reviewers draws nothing", () => {
  const { container } = render(<ReviewerAvatars reviewers={[]} />);
  expect(container).toBeEmptyDOMElement();
});

test("hovering the stack lists everyone: name and login for users, team name and org/slug for teams", () => {
  vi.useFakeTimers();
  const reviewers = [
    user("ada", "Ada Example", "https://avatars.githubusercontent.com/u/1?s=64&v=4"),
    user("bo"),
    team("acme/web", "Web Team"),
    team("acme/api"),
  ];
  render(<ReviewerAvatars reviewers={reviewers} />);
  const stack = screen.getByRole("group", { name: "Waiting on 4 reviewers" });
  fireEvent.mouseEnter(stack);
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  const tip = screen.getByRole("tooltip");
  const lines = [...tip.querySelectorAll("span.block")].map((node) => node.textContent);
  expect(lines).toEqual(["Ada Example (@ada)", "@bo", "Web Team (acme/web)", "acme/api"]);
  expect(tip).toHaveTextContent(/^Waiting on 4 reviewers/);
  expect(stack).toHaveAccessibleDescription(/Ada Example \(@ada\)/);
});

test("an avatar URL keeps an explicit size; a broken or missing one falls back to initials", () => {
  render(<ReviewerAvatars reviewers={[user("ada", "Ada Example", "https://avatars.githubusercontent.com/u/1"), team("acme/web-platform")]} />);
  const img = screen.getByTestId("reviewer-avatars").querySelector("img")!;
  expect(img.getAttribute("src")).toBe("https://avatars.githubusercontent.com/u/1?s=64");
  expect(img).toHaveAttribute("width", "20");
  fireEvent.error(img);
  expect(screen.getByTestId("reviewer-avatars").querySelector("img")).toBeNull();
  expect(screen.getByTestId("reviewer-avatars")).toHaveTextContent("AEWP");
});

test("helpers", () => {
  expect(sizedAvatar("https://avatars.githubusercontent.com/u/1?s=40&v=4")).toBe("https://avatars.githubusercontent.com/u/1?s=40&v=4");
  expect(sizedAvatar("data:image/svg+xml,%3Csvg%3E")).toBe("data:image/svg+xml,%3Csvg%3E");
  expect(initials(user("octo"))).toBe("OC");
  expect(initials(team("acme/web-platform"))).toBe("WP");
  expect(reviewerLine(user("octo", "Octo Example"))).toBe("Octo Example (@octo)");
});
