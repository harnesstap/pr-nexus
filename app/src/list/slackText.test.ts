import { expect, test } from "vitest";
import { shortTime, slackText } from "./slackText";

test("slackText turns mrkdwn into plain text", () => {
  expect(slackText("<https://x.test/a|label> <https://x.test/b> <#C0123456789|eng> <@U0ACME0001> <!here> a &lt;b&gt; &amp; c")).toBe(
    "label https://x.test/b #eng @U0ACME0001 @here a <b> & c",
  );
});

test("shortTime is relative under a day", () => {
  const now = Date.parse("2026-01-02T12:00:00Z");
  expect(shortTime(null, now)).toBe("");
  expect(shortTime("2026-01-02T11:59:50Z", now)).toBe("just now");
  expect(shortTime("2026-01-02T11:55:00Z", now)).toBe("5 min ago");
  expect(shortTime("2026-01-02T09:00:00Z", now)).toBe("3 h ago");
});
