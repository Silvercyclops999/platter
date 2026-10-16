import { useRef } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { help } from "../stores/help";
import { guides, useGuides, type GuideId } from "../stores/guides";
import { guideById, type Guide } from "./guides";
import "./guides.css";

function GuideBody({ guide }: { guide: Guide }) {
  const step = useGuides((x) => x.progress[guide.id]?.step ?? 0);
  const heading = useRef<HTMLHeadingElement>(null);
  const total = guide.steps.length;
  const atEnd = step >= total;
  const cur = atEnd ? null : guide.steps[step];
  const go = (n: number) => {
    guides.goTo(guide.id, n);
    setTimeout(() => heading.current?.focus(), 0);
  };
  return (
    <>
      <p className="guide__guide">{guide.title}</p>
      <h2 className="guide__title" ref={heading} tabIndex={-1}>
        {cur ? cur.title : "What's next"}
      </h2>
      {cur ? (
        <p className="guide__count" aria-live="polite">
          Step {step + 1} of {total}
        </p>
      ) : null}
      <div className="guide__body">
        {cur ? (
          <>
            <p className="guide__text">{cur.text}</p>
            {cur.Body && (
              <div className="guide__work">
                <cur.Body />
              </div>
            )}
            {cur.help && (
              <button type="button" className="learn-more" onClick={() => help.open(cur.help![0], cur.help![1])}>
                More about this
              </button>
            )}
          </>
        ) : (
          <>
            <p className="guide__text" role="status">
              {guide.doneText}
            </p>
            <ul className="guide__next">
              {guide.next.map((n) => (
                <li key={n.label}>
                  <Button
                    size="sm"
                    onClick={() => {
                      guides.finish(guide.id);
                      n.run();
                    }}
                  >
                    {n.label}
                  </Button>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
      <div className="guide__actions">
        {cur ? (
          <Button variant="ghost" onClick={() => guides.skip(guide.id)}>
            Skip guide
          </Button>
        ) : (
          <span />
        )}
        <span className="guide__nav">
          {step > 0 && (
            <Button variant="ghost" onClick={() => go(step - 1)}>
              Back
            </Button>
          )}
          {cur ? (
            <Button variant="primary" onClick={() => go(step + 1)}>
              {step + 1 === total ? "Finish" : "Next"}
            </Button>
          ) : (
            <Button variant="primary" onClick={() => guides.finish(guide.id)}>
              Done
            </Button>
          )}
        </span>
      </div>
    </>
  );
}

/**
 * The open setup guide. Closing it (Escape, or outside) keeps the step, so
 * it opens there next time; Skip guide stops it offering itself.
 */
export function GuideHost() {
  const active = useGuides((x) => x.active);
  const guide = active ? guideById(active as GuideId) : null;
  return (
    <Dialog open={!!guide} onClose={guides.close} label={guide ? `Setup guide: ${guide.title}` : "Setup guide"} className="confirm guide">
      {guide && <GuideBody guide={guide} />}
    </Dialog>
  );
}
