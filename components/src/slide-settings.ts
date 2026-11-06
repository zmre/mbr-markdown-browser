/**
 * Deck settings for `<mbr-slides>`: the curated Reveal.js options a
 * repository (`[slides]` in `.mbr/config.toml`) and a note (`slides:` in its
 * frontmatter) may set, and the pure merge that turns them into Reveal's
 * configuration.
 *
 * Precedence, lowest first: {@link DEFAULT_SLIDE_SETTINGS} (what mbr hardcoded
 * before these options existed), the config, the note, then mbr's fixed
 * settings, which nothing can override. A slide's own `data-transition` /
 * `data-background-*` attributes still beat all of it, because Reveal reads
 * those per slide.
 *
 * The wire contract is `crates/mbr-core/src/slides.rs` (`SlideSettings`),
 * emitted by `_head.html` as `__MBR_CONFIG__.slides` and `.slidesPage`. The
 * server validates both and resolves both background URLs; the checks here
 * only guard against a custom `_head.html` emitting something else, and a key
 * they drop is reported with `console.warn`.
 */

export const TRANSITIONS = ['none', 'fade', 'slide', 'convex', 'concave', 'zoom'] as const
export const TRANSITION_SPEEDS = ['default', 'fast', 'slow'] as const
export const SLIDE_NUMBER_FORMATS = ['h.v', 'h/v', 'c', 'c/t'] as const

export type Transition = (typeof TRANSITIONS)[number]
export type TransitionSpeed = (typeof TRANSITION_SPEEDS)[number]
export type SlideNumber = boolean | (typeof SLIDE_NUMBER_FORMATS)[number]

/** Every key optional: absent means "not set at this level". */
export interface SlideSettings {
  transition?: Transition
  transition_speed?: TransitionSpeed
  background_transition?: Transition
  slide_number?: SlideNumber
  controls?: boolean
  progress?: boolean
  background_image?: string
  background_size?: string
  background_opacity?: number
}

/**
 * The deck as it was before these settings existed: Reveal's defaults except
 * for these three, which mbr always passed.
 */
export const DEFAULT_SLIDE_SETTINGS: Readonly<SlideSettings> = Object.freeze({
  transition: 'slide',
  controls: true,
  progress: true,
})

const oneOf =
  <T>(values: readonly T[]) =>
  (v: unknown): v is T =>
    values.includes(v as T)
const isBool = (v: unknown): v is boolean => typeof v === 'boolean'
const isUrl = (v: unknown): v is string => typeof v === 'string' && v.trim() !== ''

/**
 * Every value `slide_number` may be written as, in the order a form offers
 * them. The server sends the booleans as booleans; the quoted spellings are
 * what the properties form writes, and the server reads them as booleans.
 */
export const SLIDE_NUMBER_VALUES = ['true', 'false', ...SLIDE_NUMBER_FORMATS] as const

/** One validator per key; also the list of keys that exist. */
const VALIDATORS: { [K in keyof Required<SlideSettings>]: (v: unknown) => boolean } = {
  transition: oneOf(TRANSITIONS),
  transition_speed: oneOf(TRANSITION_SPEEDS),
  background_transition: oneOf(TRANSITIONS),
  slide_number: (v) => isBool(v) || oneOf(SLIDE_NUMBER_FORMATS)(v),
  controls: isBool,
  progress: isBool,
  background_image: isUrl,
  background_size: isUrl,
  background_opacity: (v) => typeof v === 'number' && v >= 0 && v <= 1,
}

/** snake_case setting → Reveal's camelCase option, for the Reveal options only. */
const REVEAL_OPTION: Partial<Record<keyof SlideSettings, string>> = {
  transition: 'transition',
  transition_speed: 'transitionSpeed',
  background_transition: 'backgroundTransition',
  slide_number: 'slideNumber',
  controls: 'controls',
  progress: 'progress',
}

/**
 * The valid keys of `raw` (anything else is dropped with a warning naming
 * `source`). Never throws: a bad deck setting must not stop the deck.
 */
export function readSlideSettings(raw: unknown, source: string): SlideSettings {
  if (raw === undefined || raw === null) return {}
  if (typeof raw !== 'object' || Array.isArray(raw)) {
    console.warn(`[mbr-slides] ignoring ${source}: expected an object`)
    return {}
  }
  const settings: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(raw)) {
    const valid = (VALIDATORS as Record<string, ((v: unknown) => boolean) | undefined>)[key]
    if (valid?.(value)) {
      settings[key] = value
    } else {
      console.warn(`[mbr-slides] ignoring ${source}.${key}:`, value)
    }
  }
  return settings as SlideSettings
}

/** Defaults, then `config`, then `page`, key by key. */
export function mergeSlideSettings(config: SlideSettings, page: SlideSettings): SlideSettings {
  return { ...DEFAULT_SLIDE_SETTINGS, ...config, ...page }
}

/**
 * A deck's settings from the two raw objects the page carries
 * (`__MBR_CONFIG__.slides`, `.slidesPage`): each validated, then merged.
 */
export function deckSettings(config: unknown, page: unknown): SlideSettings {
  return mergeSlideSettings(
    readSlideSettings(config, 'slides'),
    readSlideSettings(page, 'frontmatter slides')
  )
}

/**
 * Reveal's configuration for a deck: the settings as Reveal options, with
 * `fixed` (mbr's own requirements: `hash`, `history`, plugins, …) spread last
 * so no setting can replace one of them. The background settings are not
 * Reveal options; {@link applyDefaultBackground} applies them.
 */
export function revealConfig(
  settings: SlideSettings,
  fixed: Record<string, unknown>
): Record<string, unknown> {
  const options: Record<string, unknown> = {}
  for (const [key, option] of Object.entries(REVEAL_OPTION)) {
    const value = settings[key as keyof SlideSettings]
    if (value !== undefined) options[option] = value
  }
  return { ...options, ...fixed }
}

/**
 * The attributes that give a slide a background of its own. Reveal's
 * `data-background-size`/`-opacity`/`-transition` alone are not one.
 */
const OWN_BACKGROUND_ATTRS = [
  'data-background',
  'data-background-image',
  'data-background-color',
  'data-background-gradient',
  'data-background-video',
  'data-background-iframe',
] as const

/**
 * Gives every top-level slide of `slides` (Reveal's `.slides` element) that
 * has no background of its own the deck's default background image, with its
 * size and opacity when set. A slide with its own background keeps it: the
 * default is not layered under it. Returns how many slides got it.
 *
 * Per-slide attributes rather than Reveal's `parallaxBackgroundImage`, which
 * scrolls one image across the whole deck and sits under every slide.
 */
export function applyDefaultBackground(slides: Element, settings: SlideSettings): number {
  const image = settings.background_image
  if (!image) return 0
  const bare = Array.from(slides.children).filter(
    (el) =>
      el.tagName === 'SECTION' && !OWN_BACKGROUND_ATTRS.some((attr) => el.hasAttribute(attr))
  )
  for (const section of bare) {
    section.setAttribute('data-background-image', image)
    if (settings.background_size) {
      section.setAttribute('data-background-size', settings.background_size)
    }
    if (settings.background_opacity !== undefined) {
      section.setAttribute('data-background-opacity', String(settings.background_opacity))
    }
  }
  return bare.length
}
