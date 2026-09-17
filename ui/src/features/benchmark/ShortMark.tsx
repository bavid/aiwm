/** Marks a run whose tok/s covers fewer generated tokens than a full pass, so
 *  it cannot be compared with the rest.
 *
 *  The reason is real, visually-hidden text rather than a `title=` tooltip: a
 *  tooltip never reaches keyboard users, is not announced reliably by screen
 *  readers, and does not exist at all on touch. */
export function ShortMark() {
  return (
    <span className="bench__short">
      short
      <span className="bench__sr"> — stopped early, so this rate is not comparable</span>
    </span>
  );
}
