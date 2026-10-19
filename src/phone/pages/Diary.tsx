/**
 * The listening diary on the phone. The computer's diary page already sits
 * in one column (month sections, filters, search, the entry editor), so the
 * phone shows it in its own margins with finger-sized controls.
 */
import { lazy } from "react";
import { Borrowed } from "./kit";

const DiaryPage = lazy(() => import("../../pages/DiaryPage").then((m) => ({ default: m.DiaryPage })));

export function PhoneDiary({ edit }: { edit?: string }) {
  return (
    <Borrowed name="diary">
      <DiaryPage key={edit ?? "diary"} edit={edit} />
    </Borrowed>
  );
}
