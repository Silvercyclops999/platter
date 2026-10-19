/**
 * The library doctor on the phone: what's worth fixing in the library,
 * each with its fix, Ignore and Why. Every fix goes through the server, so
 * it works from the phone too; the computer's page is a single list, shown
 * in the phone's own margins with finger-sized controls.
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
