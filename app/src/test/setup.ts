import { cleanup, configure } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";

// A full ListScreen render takes about 1.5 s on a busy CI runner, past the
// 1 s default of findBy* and waitFor.
configure({ asyncUtilTimeout: 5_000 });

afterEach(() => {
  cleanup();
});
