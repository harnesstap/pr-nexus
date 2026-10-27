import { expect, test } from "vitest";
import { channelLabel } from "./api";

test("channels read as #name and DMs as DM, never #D…", () => {
  expect(
    channelLabel({ channel_id: "C0123456789", channel_name: "reviews" }),
  ).toBe("#reviews");
  expect(
    channelLabel({ channel_id: "C0123456789", channel_name: "#reviews" }),
  ).toBe("#reviews");
  expect(channelLabel({ channel_id: "C0123456789", channel_name: "" })).toBe(
    "#C0123456789",
  );
  expect(
    channelLabel({ channel_id: "D0123456789", channel_name: "D0123456789" }),
  ).toBe("DM");
  expect(channelLabel({ channel_id: "D0123456789", channel_name: "" })).toBe(
    "DM",
  );
  expect(channelLabel({ channel_id: "D0123456789", channel_name: "ada" })).toBe(
    "DM · ada",
  );
});

test("members read as @name: Slack delivers them as a direct message", () => {
  expect(channelLabel({ channel_id: "U0123456789", channel_name: "Grace Hopper" })).toBe("@Grace Hopper");
  expect(channelLabel({ channel_id: "W0123456789", channel_name: "@grace" })).toBe("@grace");
  expect(channelLabel({ channel_id: "U0123456789", channel_name: "" })).toBe("@U0123456789");
});
