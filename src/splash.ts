const SPLASH_KEY = "flow.splash.seen";

/** Plays the welcome sequence once per window session, then removes its overlay. */
export function startSplashScreen(): void {
  const splash = document.getElementById("splash-screen");
  const app = document.getElementById("app-shell");
  if (!splash || !app) return;

  let alreadySeen = false;
  try { alreadySeen = sessionStorage.getItem(SPLASH_KEY) === "1"; }
  catch { /* A disabled storage service only means the animation may replay. */ }

  if (alreadySeen) {
    splash.remove();
    return;
  }

  app.inert = true;
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let finished = false;
  const finish = (): void => {
    if (finished) return;
    finished = true;
    app.inert = false;
    splash.remove();
    try { sessionStorage.setItem(SPLASH_KEY, "1"); } catch { /* No persistence available. */ }
  };

  // The fade transition is CSS driven; the timeout also handles suspended animations.
  window.setTimeout(() => {
    splash.classList.add("splash-leaving");
    splash.addEventListener("transitionend", finish, { once: true });
    window.setTimeout(finish, reducedMotion ? 80 : 650);
  }, reducedMotion ? 80 : 1900);
}
