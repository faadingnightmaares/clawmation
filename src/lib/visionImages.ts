export const MAX_IMAGE_CANDIDATES = 8;

export function imageCandidates(
  primary: string | null | undefined,
  alternatives: readonly string[] | null | undefined,
): string[] {
  const seen = new Set<string>();
  return [primary ?? "", ...(alternatives ?? [])]
    .map((path) => path.trim())
    .filter((path) => path.length > 0 && !seen.has(path) && Boolean(seen.add(path)));
}

export function splitImageCandidates(candidates: readonly string[]): {
  primary: string;
  alternatives: string[];
} {
  const normalized = imageCandidates(candidates[0], candidates.slice(1));
  return {
    primary: normalized[0] ?? "",
    alternatives: normalized.slice(1),
  };
}

export function appendImageCandidate(
  candidates: readonly string[],
  path: string,
): { candidates: string[]; added: boolean; full: boolean } {
  const normalized = imageCandidates(candidates[0], candidates.slice(1));
  const nextPath = path.trim();
  if (!nextPath || normalized.includes(nextPath)) {
    return { candidates: normalized, added: false, full: false };
  }
  if (normalized.length >= MAX_IMAGE_CANDIDATES) {
    return { candidates: normalized, added: false, full: true };
  }
  return {
    candidates: [...normalized, nextPath],
    added: true,
    full: false,
  };
}

export function removeImageCandidate(
  candidates: readonly string[],
  index: number,
): string[] {
  return imageCandidates(candidates[0], candidates.slice(1)).filter(
    (_, candidateIndex) => candidateIndex !== index,
  );
}
