import { parseChangelog, sortReleases, unreleasedRelease, type ChangelogRelease } from "./changelog";

// Vite inlines these at build time, so the notes match the installed build and work offline.
const versionFiles = import.meta.glob<string>("../../../.changes/v*.md", { query: "?raw", import: "default", eager: true });
const fragmentFiles = import.meta.glob<string>("../../../.changes/unreleased/*.yaml", { query: "?raw", import: "default", eager: true });

/** Every released version embedded in this build, newest first. */
export const RELEASES: ChangelogRelease[] = sortReleases(Object.values(versionFiles).flatMap(parseChangelog));

/** Fragments not released yet (dev builds and PR branches); null on a release build. */
export const UNRELEASED: ChangelogRelease | null = unreleasedRelease(Object.values(fragmentFiles));
