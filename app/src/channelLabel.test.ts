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
