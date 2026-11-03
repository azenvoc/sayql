// Validation rules for saved skills, shared by the Skills settings and the dev mock backend.
// Lives in skills (not ask) so settings → skills doesn't depend on the Ask box.
import type { Skill } from "../../api";

/** Slugs taken by built-in Ask commands. */
export const BUILTIN_SLUGS = [
  "investigate",
  "followup",
  "new",
  "explain",
  "fix",
  "improve",
] as const;
/** What skills can't be called: the built-ins and `/agent`, the old name of /investigate. Must match BUILTIN_COMMANDS in src-tauri/src/store/skills.rs. */
export const RESERVED_SLUGS: readonly string[] = [...BUILTIN_SLUGS, "agent"];

export const SKILL_LIMITS = { slug: 32, description: 100, instruction: 1000 };

/** What's wrong with a skill before saving, or null. Same rules as `validate` in src-tauri/src/commands/skills.rs. */
export function skillError(
  skill: Skill,
  existing: Skill[],
  original: string | null,
): string | null {
  const slug = skill.slug.trim().replace(/^\//, "");
  if (!slug) return "Give the skill a name.";
  if (slug.length > SKILL_LIMITS.slug) return `Keep the name to ${SKILL_LIMITS.slug} characters.`;
  if (!/^[a-z0-9-]+$/.test(slug))
    return "Use only lowercase letters, numbers and dashes in the name.";
  if (RESERVED_SLUGS.includes(slug)) return `/${slug} is a built-in command. Pick another name.`;
  if (existing.some((s) => s.slug === slug && s.slug !== original))
    return `You already have a skill called /${slug}.`;
  if ([...skill.description.trim()].length > SKILL_LIMITS.description)
    return `Keep the description to ${SKILL_LIMITS.description} characters.`;
  const instruction = skill.instruction.trim();
  if (!instruction) return "Write the instruction.";
  if ([...instruction].length > SKILL_LIMITS.instruction)
    return `Keep the instruction to ${SKILL_LIMITS.instruction} characters.`;
  return null;
}

/** Suggestions shown while there are no skills; not installed until the person saves one. */
export const EXAMPLE_SKILLS: Skill[] = [
  {
    slug: "pii-safe",
    description: "Leave out personal data",
    instruction: "Never select email, phone or address columns.",
  },
  {
    slug: "this-month",
    description: "Only this month",
    instruction: "Only include rows from the current calendar month.",
  },
  { slug: "small", description: "At most 20 rows", instruction: "Return at most 20 rows." },
];
