import type { McpCapability, McpSettings, McpToolInfo } from "../api";

/** The capability switch, including parents that gate Slack private search and sends. */
export function categoryEnabled(settings: McpSettings, category: McpCapability): boolean {
  switch (category) {
    case "reads":
      return settings.reads;
    case "local_writes":
      return settings.local_writes;
    case "slack_reads":
      return settings.slack_reads;
    case "slack_private_search":
      return settings.slack_reads && settings.slack_private_search;
    case "slack_sends":
      return settings.slack_reads && settings.slack_sends;
    default: {
      const never: never = category;
      return never;
    }
  }
}

/** The per-tool switch, ignoring whether the capability itself is on. */
export function toolOn(tool: McpToolInfo, settings: Pick<McpSettings, "disabled_tools" | "enabled_slack_send_tools">): boolean {
  if (settings.disabled_tools.includes(tool.name)) {
    return false;
  }
  if (tool.defaults_on) {
    return true;
  }
  return settings.enabled_slack_send_tools.includes(tool.name);
}

export function toolSummary(on: number, total: number): string {
  return `${on} of ${total} ${total === 1 ? "tool" : "tools"}`;
}

function sorted(names: string[]): string[] {
  return [...names].sort();
}

/** The settings patch that turns one tool on or off without touching its capability. */
export function withTool(
  settings: McpSettings,
  tool: McpToolInfo,
  on: boolean,
): Pick<McpSettings, "disabled_tools" | "enabled_slack_send_tools"> {
  const disabled = settings.disabled_tools.filter((name) => name !== tool.name);
  const enabled = settings.enabled_slack_send_tools.filter((name) => name !== tool.name);
  if (on) {
    if (!tool.defaults_on) {
      enabled.push(tool.name);
    }
  } else if (tool.defaults_on) {
    disabled.push(tool.name);
  }
  return { disabled_tools: sorted(disabled), enabled_slack_send_tools: sorted(enabled) };
}

/** The settings patch for All or None inside one category. */
export function withTools(
  settings: McpSettings,
  tools: McpToolInfo[],
  on: boolean,
): Pick<McpSettings, "disabled_tools" | "enabled_slack_send_tools"> {
  const names = new Set(tools.map((tool) => tool.name));
  const disabled = settings.disabled_tools.filter((name) => !names.has(name));
  const enabled = settings.enabled_slack_send_tools.filter((name) => !names.has(name));
  if (on) {
    for (const tool of tools) {
      if (!tool.defaults_on) {
        enabled.push(tool.name);
      }
    }
  } else {
    for (const tool of tools) {
      if (tool.defaults_on) {
        disabled.push(tool.name);
      }
    }
  }
  return { disabled_tools: sorted(disabled), enabled_slack_send_tools: sorted(enabled) };
}
