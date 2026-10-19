/**
 * Create a Blend on the phone: pick the people to combine tastes with. The
 * computer's page is a short form in one column, shown in the phone's own
 * margins with finger-sized controls.
 */
import { lazy } from "react";
import { Borrowed } from "./kit";

const BlendCreatePage = lazy(() => import("../../pages/BlendCreatePage").then((m) => ({ default: m.BlendCreatePage })));

export function PhoneBlendNew() {
  return (
    <Borrowed name="blendNew">
      <BlendCreatePage />
    </Borrowed>
  );
}
