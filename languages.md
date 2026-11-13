# Languages

kadr's interface comes in English and Russian. English is the source: every string is written in English first, and the other languages are translated from it. This manual is in English only.

## Choosing a language

Settings → General → **Language**. In `settings.toml` it's the `language` key. A string a language doesn't have yet shows in English.

## How much is translated

| Language | File | Translated |
|---|---|---|
| English | [en.json](https://github.com/evlaso/kadr/blob/main/data/i18n/en.json) | source |
| Russian | [ru.json](https://github.com/evlaso/kadr/blob/main/data/i18n/ru.json) | 100% |

## Helping with a translation

Corrections to a language that's already here and new languages are both welcome.

- **To fix a string:** find it in the language's file, change the text on the right of the colon, and open a pull request. If you'd rather not, [open an issue](https://github.com/evlaso/kadr/issues) with the text as it is now and as it should be.
- **To add a language:** copy [en.json](https://github.com/evlaso/kadr/blob/main/data/i18n/en.json) to a file named by the language's code (`de.json`, `uk.json`), translate the text on the right of each colon, and open a pull request. It doesn't have to be complete: untranslated strings show in English. The language appears in the settings once a maintainer adds it to the list.

A few rules keep a translation working:

- Leave the keys on the left as they are (`"editor.save"`), and translate only the text on the right.
- Keep every `{}` in a string: kadr puts a number or a name there. Move it where your language needs it.
- Use the words the interface already uses for the same thing; the [glossary](glossary.md) defines them in English.
- Keep it short: buttons and bars have little room, and a longer label may fold away.
