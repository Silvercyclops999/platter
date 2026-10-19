/**
 * Your shelf on the phone: the records you own on vinyl, CD or cassette and
 * the Discogs import. The computer's page is already one column, so the
 * phone shows it in its own margins with finger-sized controls.
 */
import { lazy } from "react";
import { Borrowed } from "./kit";

const ShelfPage = lazy(() => import("../../pages/ShelfPage").then((m) => ({ default: m.ShelfPage })));

export function PhoneShelf() {
  return (
    <Borrowed name="shelf">
      <ShelfPage />
    </Borrowed>
  );
}
