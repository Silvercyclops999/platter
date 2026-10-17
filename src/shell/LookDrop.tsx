/**
 * Drop a .platter-look (or theme .json) file anywhere on the window to add
 * it to your looks. The file's text goes to the core, which checks it and
 * saves it in the themes folder; nothing in it runs.
 */
import { useEffect, useState } from "react";
import { importLookText, isLookFileName } from "../stores/looks";
import { THEME_FILE_MAX_BYTES } from "../design/themes/validate";
import { toast } from "../stores/toasts";

const hasFiles = (e: DragEvent) => !!e.dataTransfer && Array.from(e.dataTransfer.types).includes("Files");
/** A dialog has its own drop target (a cover picker, say), so the window steps aside. */
const dialogOpen = () => !!document.querySelector(".dialog-backdrop");

/** Read one dropped file and add it. Exported for tests. */
export async function addDroppedFile(file: File): Promise<void> {
  if (!isLookFileName(file.name)) {
    toast.error("Drop a .platter-look file to add a look");
    return;
  }
  if (file.size > THEME_FILE_MAX_BYTES) {
    toast.error("Couldn't add that look: the file is too big for a look (64 KB at most)");
    return;
  }
  await importLookText(await file.text());
}

export function LookDrop() {
  const [over, setOver] = useState(false);
  useEffect(() => {
    let depth = 0;
    const enter = (e: DragEvent) => {
      if (!hasFiles(e) || dialogOpen()) return;
      depth++;
      setOver(true);
    };
    const leave = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      depth = Math.max(0, depth - 1);
      if (!depth) setOver(false);
    };
    const overFn = (e: DragEvent) => {
      if (!hasFiles(e) || dialogOpen()) return;
      // Without this the window would open the file instead.
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const drop = (e: DragEvent) => {
      // A drop target inside the page (a cover picker) already took it.
      if (!hasFiles(e) || e.defaultPrevented || dialogOpen()) {
        depth = 0;
        setOver(false);
        return;
      }
      e.preventDefault();
      depth = 0;
      setOver(false);
      const file = e.dataTransfer?.files?.[0];
      if (file) void addDroppedFile(file);
    };
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragleave", leave);
    window.addEventListener("dragover", overFn);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("dragover", overFn);
      window.removeEventListener("drop", drop);
    };
  }, []);
  return over ? (
    <div className="look-drop" aria-hidden="true">
      <span className="look-drop__text">Drop a look file to add it</span>
    </div>
  ) : null;
}
