import { Shelf, ShelfItem } from "../components/Shelf";
import { backend } from "../lib/backend";
import { useLoad } from "../lib/useLoad";
import { nav } from "../stores/router";
import { AlbumCard } from "./cards";

function greeting(d = new Date()): string {
  const h = d.getHours();
  if (h < 5) return "Good night";
  if (h < 12) return "Good morning";
  if (h < 18) return "Good afternoon";
  return "Good evening";
}

export function HomePage() {
  const { data, error } = useLoad(() => backend.home(), []);
  const shelves = [
    { title: "Recently played", items: data?.recentlyPlayed },
    { title: "Jump back in", items: data?.jumpBackIn },
    { title: "Recently added", items: data?.recentlyAdded },
  ];
  return (
    <div className="page-pad home">
      <h1 className="page-title">{greeting()}</h1>
      {error && !data ? (
        <p className="page-error">Couldn't load Home: {error.message}</p>
      ) : (
        shelves.map((s) => (
          <Shelf key={s.title} title={s.title} loading={!data} onShowAll={() => nav.go({ name: "library", tab: "albums" })}>
            {s.items?.map((a) => (
              <ShelfItem key={a.id}>
                <AlbumCard album={a} />
              </ShelfItem>
            ))}
          </Shelf>
        ))
      )}
    </div>
  );
}
