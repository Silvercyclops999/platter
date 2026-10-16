import { HelpView } from "./HelpView";
import { helpSection } from "./pages";

/** One help section shown in place (inside a dialog, which covers the side panel). */
export default function InlineHelp({ page, anchor }: { page: string; anchor?: string }) {
  return <HelpView slug={page} blocks={helpSection(page, anchor)} />;
}
