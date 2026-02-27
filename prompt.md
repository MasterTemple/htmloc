
- Browsers have a feature called [Text Fragments](https://developer.mozilla.org/en-US/docs/Web/URI/Reference/Fragment/Text_fragments) which are very powerful.
- They can be used by selecting text, right clicking, and selecting "Copy link to highlight"

Create a rust library for creating text fragments from line-column pairs

- Implement the algorithm exactly as it is in Chromium
- Make sure it supports all features and handles all edge cases

You may support any additional, relevant features:
- Calculating text fragments from plain-text documents (not just HTML)

---

Yes, add the capability to optionally run on raw HTML. You may also use the line_col crate if that is faster (for resolving byte offsets)

---

Very good

- For the sake of robustness, allow the user to pass an optional robustness parameter that will not be satisfied at being unique, but will include a prefix and suffix (even though not necessary). Let the user be able to adjust how much of the prefix and suffix they want.
- Also allow the reverse process: take a text fragment and create the line-column range

---

Yes, add the following features:

- Parse a text fragment from the hash
- Make the fragment resolver handle complex whitespace (use RegEx or Chumsky or a handcrafted solution)
- When resolving the end index, make sure that you give the ending index of the text (not including the following HTML tags). Previously, `1 Peter 5:2 (ESV)` in `<p><span class=citation>1 Peter 5:2 (ESV)</span></p>` would calculate the end index at `>` in `</span></p>` instead of `)` in `(ESV)` (which is part of the matched text)
