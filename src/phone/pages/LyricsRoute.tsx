/**
 * Lyrics are never a page: reaching the route opens the player's full
 * screen lyrics and steps back to where you were.
 */
import { useEffect } from "react";
import { useRouter } from "../../stores/router";
import { phone } from "../phoneUi";
import { PageLoading } from "./common";

export function PhoneLyricsRoute() {
  useEffect(() => {
    phone.openPlayer();
    phone.openLyrics();
    const r = useRouter.getState();
    if (r.index > 0) r.back();
    else r.reset({ name: "home" });
  }, []);
  return (
    <div className="ph-page" data-phone-page="lyrics">
      <PageLoading />
    </div>
  );
}
