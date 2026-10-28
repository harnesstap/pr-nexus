import type { JiraLinkDto, ProjectDto } from "../api";

/** One JIRA reference. Opens its link when it has one; a bare key without a base URL is plain text. */
export function JiraChip({ link, onOpen }: { link: JiraLinkDto; onOpen?: (url: string) => void }) {
  const label = link.key || shortUrl(link.url);
  const look = "inline-flex max-w-40 shrink-0 items-center truncate rounded border px-1.5 font-mono text-[11px] leading-4";
  if (!link.href) {
    return (
      <span className={`${look} border-slate-700 text-slate-400`} title="Set a JIRA base URL in Settings to link bare keys">
        {label}
      </span>
    );
  }
  const href = link.href;
  return (
    <a
      href={href}
      title={href}
      className={`${look} border-sky-500/40 text-sky-200 hover:bg-sky-500/10`}
      onClick={(event) => {
        event.stopPropagation();
        if (onOpen) {
          event.preventDefault();
          onOpen(href);
        }
      }}
      target="_blank"
      rel="noreferrer"
    >
      {label}
    </a>
  );
}

function shortUrl(url: string): string {
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname}`.replace(/\/$/, "");
  } catch {
    return url;
  }
}

/** A project's JIRA chips and description, where the project is shown. */
export function ProjectInfo({ project, onOpen, className = "" }: { project: ProjectDto; onOpen?: (url: string) => void; className?: string }) {
  const description = project.description.trim();
  if (!description && project.jira_links.length === 0) {
    return null;
  }
  return (
    <div className={`flex min-w-0 items-center gap-1.5 ${className}`} data-testid={`project-info-${project.id}`}>
      {project.jira_links.map((link) => (
        <JiraChip key={link.id} link={link} onOpen={onOpen} />
      ))}
      {description && (
        <span className="min-w-0 truncate text-xs text-slate-500" title={description}>
          {description.split("\n")[0]}
        </span>
      )}
    </div>
  );
}
