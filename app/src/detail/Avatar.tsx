import { useState } from "react";
import type { Person } from "../api";
import { initials, sizedAvatar } from "../list/ReviewerAvatars";

interface Props {
  person: Person;
  /** Pixel size. */
  size?: number;
  /** Ring classes, e.g. a review state colour; defaults to the panel background. */
  ring?: string;
}

/** A user (circle) or team (rounded square) picture, with initials when it can't load. */
export function Avatar({ person, size = 24, ring = "ring-2 ring-slate-900" }: Props) {
  const [failed, setFailed] = useState(false);
  const shape = person.kind === "team" ? "rounded-md" : "rounded-full";
  const box = `relative inline-flex shrink-0 overflow-hidden ${shape} ${ring}`;
  const style = { width: size, height: size };
  if (person.avatar_url && !failed) {
    return (
      <img
        src={sizedAvatar(person.avatar_url)}
        alt=""
        width={size}
        height={size}
        style={style}
        loading="lazy"
        draggable={false}
        onError={() => setFailed(true)}
        data-person={person.login}
        className={`${box} bg-slate-800 object-cover`}
      />
    );
  }
  return (
    <span
      aria-hidden="true"
      data-person={person.login}
      style={{ ...style, fontSize: Math.max(9, Math.round(size * 0.4)) }}
      className={`${box} items-center justify-center font-semibold leading-none ${person.kind === "team" ? "bg-indigo-900 text-indigo-200" : "bg-slate-700 text-slate-200"}`}
    >
      {initials(person)}
    </span>
  );
}
