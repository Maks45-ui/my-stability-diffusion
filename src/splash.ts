const SPLASH_KEY = "flow.splash.seen";

/** Resolves after the welcome overlay has disappeared. The caller owns app access. */
export function startSplashScreen(): Promise<void> {
  return new Promise((resolve) => {
    const splash = document.getElementById("splash-screen");
    if (!splash) { resolve(); return; }

  let alreadySeen = false;
  try { alreadySeen = sessionStorage.getItem(SPLASH_KEY) === "1"; }
  catch { /* A disabled storage service only means the animation may replay. */ }

    if (alreadySeen) { splash.remove(); resolve(); return; }

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let finished = false;
  const finish = (): void => {
    if (finished) return;
    finished = true;
    splash.remove();
    try { sessionStorage.setItem(SPLASH_KEY, "1"); } catch { /* No persistence available. */ }
    resolve();
  };

  // The fade transition is CSS driven; the timeout also handles suspended animations.
  window.setTimeout(() => {
    splash.classList.add("splash-leaving");
    splash.addEventListener("transitionend", finish, { once: true });
    window.setTimeout(finish, reducedMotion ? 80 : 650);
  }, reducedMotion ? 80 : 1900);
  });
}
