import { useId, type SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 16, children, ...props }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export function FunnelIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 4h18l-7 8.5V19l-4 2v-8.5L3 4z" />
    </Svg>
  );
}

export function SearchIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </Svg>
  );
}

export function HomeIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 11 12 4l9 7" />
      <path d="M5 10v10h14V10" />
    </Svg>
  );
}

export function ClockIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </Svg>
  );
}

export function FolderIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 6h6l2 2h10v11H3z" />
    </Svg>
  );
}

export function GearIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    </Svg>
  );
}

export function RefreshIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M20 11a8 8 0 0 0-14.5-4.5L4 8" />
      <path d="M4 4v4h4" />
      <path d="M4 13a8 8 0 0 0 14.5 4.5L20 16" />
      <path d="M20 20v-4h-4" />
    </Svg>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M12 5v14M5 12h14" />
    </Svg>
  );
}

export function CommentIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 5h16v11H9l-5 4z" />
    </Svg>
  );
}

export function ExternalIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M14 4h6v6" />
      <path d="M20 4 10 14" />
      <path d="M18 14v6H4V6h6" />
    </Svg>
  );
}

const LOGO_BUBBLE =
  "M700 214H820A44 44 0 0 1 864 258V320A44 44 0 0 1 820 364H752L700 410V364A44 44 0 0 1 656 320V258A44 44 0 0 1 700 214Z";

/**
 * PR Nexus mark for dark backgrounds: the app icon without its tile (same drawing as public/favicon.svg).
 * It keeps its own colors, so `className` text colors have no effect.
 */
export function LogoIcon({ size = 16, ...props }: IconProps) {
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  return (
    <svg width={size} height={size} viewBox="222 186 662 662" aria-hidden="true" focusable="false" {...props}>
      <defs>
        <linearGradient id={`${id}-hex`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#8B5CF6" />
          <stop offset="1" stopColor="#A78BFA" />
        </linearGradient>
        <mask id={`${id}-cut`} maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024">
          <rect width="1024" height="1024" fill="#fff" />
          <path d={LOGO_BUBBLE} fill="#000" stroke="#000" strokeWidth={40} strokeLinejoin="round" />
        </mask>
      </defs>
      <path
        d="M477.4 232Q512 212 546.6 232L733.7 340Q768.3 360 768.3 400L768.3 616Q768.3 656 733.7 676L546.6 784Q512 804 477.4 784L290.3 676Q255.7 656 255.7 616L255.7 400Q255.7 360 290.3 340Z"
        fill="none"
        stroke={`url(#${id}-hex)`}
        strokeWidth={58}
        strokeLinejoin="round"
        mask={`url(#${id}-cut)`}
      />
      <path
        d="M384 396C384 474 512 466 512 548V672M640 396C640 474 512 466 512 548"
        fill="none"
        stroke="#fff"
        strokeWidth={62}
        strokeLinecap="round"
      />
      <g fill="#fff">
        <circle cx="384" cy="396" r="68" />
        <circle cx="640" cy="396" r="68" />
        <circle cx="512" cy="672" r="68" />
      </g>
      <path d={LOGO_BUBBLE} fill="#2DD4BF" />
      <g fill="#fff">
        <circle cx="712" cy="289" r="20" />
        <circle cx="760" cy="289" r="20" />
        <circle cx="808" cy="289" r="20" />
      </g>
    </svg>
  );
}

/** Slack-style mark: four colored bars when `active`, a single muted outline otherwise. */
export function SlackIcon({ active = true, size = 14 }: { active?: boolean; size?: number }) {
  const colors = active ? ["#36c5f0", "#2eb67d", "#ecb22e", "#e01e5a"] : ["#475569", "#475569", "#475569", "#475569"];
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <rect x="9" y="2" width="4" height="12" rx="2" fill={colors[0]} />
      <rect x="10" y="9" width="12" height="4" rx="2" fill={colors[1]} />
      <rect x="11" y="10" width="4" height="12" rx="2" fill={colors[2]} />
      <rect x="2" y="11" width="12" height="4" rx="2" fill={colors[3]} />
    </svg>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6 6l12 12M18 6 6 18" />
    </Svg>
  );
}

export function FolderPlusIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 6h6l2 2h10v11H3z" />
      <path d="M12 11v5M9.5 13.5h5" />
    </Svg>
  );
}

export function MoreIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="5" cy="12" r="1" />
      <circle cx="12" cy="12" r="1" />
      <circle cx="19" cy="12" r="1" />
    </Svg>
  );
}

export function ChevronDownIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m6 9 6 6 6-6" />
    </Svg>
  );
}

export function ChevronRightIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m9 6 6 6-6 6" />
    </Svg>
  );
}

export function ChevronLeftIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m15 6-6 6 6 6" />
    </Svg>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m5 12 5 5 9-10" />
    </Svg>
  );
}

export function CopyIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a1 1 0 0 1 1-1h9" />
    </Svg>
  );
}

export function SkipIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="m6 6 6 6-6 6" />
      <path d="M17 6v12" />
    </Svg>
  );
}

/** The GitHub mark, filled with the text color. */
export function GitHubIcon({ size = 16, ...props }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" focusable="false" {...props}>
      <path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z" />
    </svg>
  );
}
