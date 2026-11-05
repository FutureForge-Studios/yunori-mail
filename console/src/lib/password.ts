/** Rough, explainable strength estimate for the ≥12-character rule. */
export type Strength = { score: 0 | 1 | 2 | 3 | 4; label: string; hint: string };

export const MIN_PASSWORD = 12;

export function passwordStrength(pw: string): Strength {
  if (pw.length === 0) return { score: 0, label: "", hint: `At least ${MIN_PASSWORD} characters.` };
  if (pw.length < MIN_PASSWORD)
    return { score: 1, label: "Too short", hint: `${MIN_PASSWORD - pw.length} more character${MIN_PASSWORD - pw.length === 1 ? "" : "s"} to go.` };
  const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((r) => r.test(pw)).length;
  const repeats = /(.)\1{3,}/.test(pw) || new Set(pw).size < pw.length / 3;
  if (repeats) return { score: 2, label: "Fair", hint: "Lots of repetition — mix it up a little." };
  if (pw.length >= 20 || (pw.length >= 16 && classes >= 3)) return { score: 4, label: "Strong", hint: "Good — a long passphrase is the best kind." };
  if (classes >= 3 || pw.length >= 16) return { score: 3, label: "Good", hint: "A few more words would make it stronger." };
  return { score: 2, label: "Fair", hint: "Longer is better: try a short phrase of unrelated words." };
}
