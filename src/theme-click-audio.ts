const CLICK_SOUND = "/assets/boykisser-click.mp3";
const VOICE_COUNT = 4;

/** Plays mouse clicks only while the Boykisser theme is selected. */
export class ThemeClickAudio {
  private enabled = false;
  private voices: HTMLAudioElement[] = [];
  private nextVoice = 0;

  constructor() {
    // Capture also sees clicks inside dialogs and controls that stop bubbling.
    document.addEventListener("click", this.onClick, true);
  }

  setEnabled(enabled: boolean): void {
    if (this.enabled === enabled) return;
    this.enabled = enabled;
    if (enabled) {
      this.voices = Array.from({ length: VOICE_COUNT }, () => {
        const audio = new Audio(CLICK_SOUND);
        audio.preload = "auto";
        audio.volume = 0.55;
        return audio;
      });
    } else {
      for (const voice of this.voices) {
        voice.pause();
        voice.removeAttribute("src");
        voice.load();
      }
      this.voices = [];
      this.nextVoice = 0;
    }
  }

  private readonly onClick = (event: MouseEvent): void => {
    if (!this.enabled || document.documentElement.dataset.theme !== "boykisser" || event.detail === 0) return;
    // The click that exits the theme must be silent, even during capture.
    const themeButton = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-theme-option]") : null;
    if (themeButton && themeButton.dataset.themeOption !== "boykisser") return;

    const voice = this.voices[this.nextVoice];
    if (!voice) return;
    this.nextVoice = (this.nextVoice + 1) % this.voices.length;
    voice.pause();
    voice.currentTime = 0;
    void voice.play().catch(() => { /* Audio can be blocked by system settings. */ });
  };
}
