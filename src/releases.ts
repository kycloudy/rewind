export type Release = { tag: string; name: string; body: string; date: string };

type GithubRelease = {
  tag_name: string;
  name: string | null;
  body: string | null;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
};

/** Published releases of kycloudy/rewind, newest first. */
export async function fetchReleases(): Promise<Release[]> {
  const res = await fetch("https://api.github.com/repos/kycloudy/rewind/releases?per_page=20", {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!res.ok) throw new Error(`GitHub returned ${res.status}`);
  const all: GithubRelease[] = await res.json();
  return all
    .filter((r) => !r.draft && r.body?.trim())
    .sort((a, b) => Date.parse(b.published_at ?? "") - Date.parse(a.published_at ?? ""))
    .map((r) => ({ tag: r.tag_name, name: r.name || r.tag_name, body: r.body!, date: r.published_at ?? "" }));
}
