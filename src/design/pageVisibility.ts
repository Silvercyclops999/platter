/**
 * Whether the page can be seen. While it can't (window minimised, hidden to
 * the tray, or behind another tab in a browser), `<html data-page-hidden>`
 * pauses every CSS animation and JS loops that only draw stop.
 */
import { useSyncExternalStore } from "react";

function read(doc: Document): boolean {
  return doc.visibilityState === "hidden" || doc.hidden === true;
}

function apply(doc: Document) {
  doc.documentElement.dataset.pageHidden = read(doc) ? "true" : "false";
}

/** True while the page is hidden. */
export function isPageHidden(doc: Document = document): boolean {
  return read(doc);
}

function subscribe(cb: () => void): () => void {
  document.addEventListener("visibilitychange", cb);
  return () => document.removeEventListener("visibilitychange", cb);
}

/** The page's visibility as React state. */
export function usePageVisible(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => !read(document),
    () => true,
  );
}

/** Mirror visibility onto `<html data-page-hidden>`. Returns a cleanup. */
export function installPageVisibility(doc: Document = document): () => void {
  apply(doc);
  const on = () => apply(doc);
  doc.addEventListener("visibilitychange", on);
  return () => doc.removeEventListener("visibilitychange", on);
}
