import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, ListRow, PrDetail } from "../api";
import { PrDetailDialog, requirementLabel } from "./PrDetailDialog";

const row = {
  id: 7,
  number: 42,
  title: "Split invoices",
  url: "https://github.com/acme/widgets/pull/42",
  repo: "acme/widgets",
} as ListRow;

const detail: PrDetail = {
  id: 7,
  readiness: "unknown",
  readiness_notes: ["Approvals could not be matched to every code-owner team."],
  head_sha: "0123456789abcdef",
  base_ref: "main",
  codeowners_path: ".github/CODEOWNERS",
  codeowners_error: null,
  codeowners_required: "true",
  codeowners_required_reason: "A branch rule requires code-owner review.",
  code_owner_review_pending: null,
  mandatory_teams: ["@acme/billing", "@acme/platform"],
  optional_teams: [],
  approved_teams: ["@acme/platform"],
  candidate_teams: ["@acme/billing"],
  attribution_unknown: [{ team: "@acme/billing", reasons: ["team members are not visible"] }],
  requested_teams: ["@acme/billing"],
  users_ignored: [],
  rationale: ["billing/invoice.rs is owned by @acme/billing"],
  files: [
    { path: "billing/invoice.rs", teams: ["@acme/billing"] },
    { path: "README.md", teams: ["@acme/platform"] },
  ],
  files_truncated: false,
  rules_error: null,
  protection: "Classic branch protection is only visible to repository admins.",
  from_cache: true,
};

test("loads the detail and lists owners, gaps, and files", async () => {
  const prDetail = vi.fn(() => Promise.resolve(detail));
  const onClose = vi.fn();
  render(<PrDetailDialog api={{ prDetail } as unknown as Api} row={row} onClose={onClose} onOpen={() => undefined} />);
  expect(screen.getByText("Checking code owners…")).toBeInTheDocument();
  expect(await screen.findByText("Unknown")).toBeInTheDocument();
  expect(prDetail).toHaveBeenCalledWith(7);
  expect(screen.getByText(/code-owner review required, status unknown/)).toBeInTheDocument();
  expect(screen.getByText("@acme/billing: team members are not visible")).toBeInTheDocument();
  expect(screen.getByText("billing/invoice.rs")).toBeInTheDocument();
  expect(screen.getByText(/only visible to repository admins/)).toBeInTheDocument();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});

test("shows the error when the detail cannot load", async () => {
  const api = { prDetail: () => Promise.reject(new Error("rate limited")) } as unknown as Api;
  render(<PrDetailDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} />);
  expect(await screen.findByText(/rate limited/)).toBeInTheDocument();
});

test("requirement labels separate pending, satisfied, and unknown", () => {
  expect(requirementLabel({ codeowners_required: "false", code_owner_review_pending: null })).toBe("code-owner review not required");
  expect(requirementLabel({ codeowners_required: "unknown", code_owner_review_pending: null })).toBe("code-owner requirement unknown");
  expect(requirementLabel({ codeowners_required: "true", code_owner_review_pending: true })).toBe("code-owner review required, still pending");
  expect(requirementLabel({ codeowners_required: "true", code_owner_review_pending: false })).toBe("code-owner review required, satisfied");
});
