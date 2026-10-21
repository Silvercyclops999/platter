/**
 * The library doctor on the phone: what's worth fixing in the library,
 * each with its fix, Ignore and Why. Fixes that go through the server work
 * from the phone too; tag fixes write into the music folder, so those say to
 * use the computer. The computer's page is a single list, shown in the
 * phone's own margins with finger-sized controls. It opens from the end of
 * Library while there's something to fix, and from Settings, Library.
 */
import { lazy } from "react";
import { Borrowed } from "./kit";

const DoctorPage = lazy(() => import("../../pages/DoctorPage").then((m) => ({ default: m.DoctorPage })));

export function PhoneDoctor() {
  return (
    <Borrowed name="doctor">
      <DoctorPage />
    </Borrowed>
  );
}
