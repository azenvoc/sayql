import { useEffect, useRef, useState } from "react";
import { Pencil, Plus, ScrollText, Trash2 } from "lucide-react";
import type { Skill } from "../../api";
import { Alert, Button, Field, IconButton, TextInput } from "../../components/ui";
import { errorText } from "../../lib/errors";
import { EXAMPLE_SKILLS, SKILL_LIMITS, skillError } from "./rules";
import type { SkillsApi } from "./useSkills";
import styles from "./SkillsSection.module.css";

/** What the editor holds; `original` is the slug before an edit, null for a new skill. */
interface Editing {
  skill: Skill;
  original: string | null;
}

const blank: Skill = { slug: "", description: "", instruction: "" };

/** The Skills part of Settings: list, create, edit and delete. Saves each skill right away. */
export function SkillsSection({
  skills,
  startCreating,
}: {
  skills: SkillsApi;
  startCreating?: boolean;
}) {
  const [editing, setEditing] = useState<Editing | null>(
    startCreating ? { skill: blank, original: null } : null,
  );
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (startCreating) ref.current?.scrollIntoView({ block: "nearest" });
  }, [startCreating]);

  const edit = (skill: Skill, original: string | null) => {
    setError(null);
    setEditing({ skill, original });
  };
  const update = (patch: Partial<Skill>) =>
    editing && setEditing({ ...editing, skill: { ...editing.skill, ...patch } });

  const save = async () => {
    if (!editing) return;
    const problem = skillError(editing.skill, skills.skills, editing.original);
    if (problem) return setError(problem);
    try {
      await skills.save(editing.skill, editing.original);
      setEditing(null);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const remove = async (slug: string) => {
    try {
      await skills.remove(slug);
      if (editing?.original === slug) setEditing(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div ref={ref} className={styles.section}>
      {skills.skills.length > 0 ? (
        <ul className={styles.list}>
          {skills.skills.map((s) => (
            <li key={s.slug} className={styles.row}>
              <ScrollText size={14} className={styles.icon} />
              <span className={styles.slug}>/{s.slug}</span>
              <span className={styles.description}>{s.description || s.instruction}</span>
              <IconButton
                icon={Pencil}
                label={`Edit /${s.slug}`}
                size="sm"
                onClick={() => edit(s, s.slug)}
              />
              <IconButton
                icon={Trash2}
                label={`Delete /${s.slug}`}
                size="sm"
                onClick={() => remove(s.slug)}
              />
            </li>
          ))}
        </ul>
      ) : (
        !editing && (
          <div className={styles.examples}>
            <span className={styles.examplesLabel}>No skills yet. Start from an example:</span>
            {EXAMPLE_SKILLS.map((ex) => (
              <button
                key={ex.slug}
                type="button"
                className={styles.example}
                title={ex.instruction}
                onClick={() => edit(ex, null)}
              >
                <span className={styles.slug}>/{ex.slug}</span> {ex.description}
              </button>
            ))}
          </div>
        )
      )}

      {editing ? (
        <div className={styles.editor}>
          <Field label="Name" hint="Lowercase letters, numbers and dashes.">
            {(id) => (
              <div className={styles.slugInput}>
                <span className={styles.slash}>/</span>
                <TextInput
                  id={id}
                  mono
                  autoFocus
                  maxLength={SKILL_LIMITS.slug}
                  value={editing.skill.slug}
                  placeholder="pii-safe"
                  onChange={(e) =>
                    update({ slug: e.target.value.toLowerCase().replace(/\s/g, "-") })
                  }
                />
              </div>
            )}
          </Field>
          <Field label="Description" optional>
            {(id) => (
              <TextInput
                id={id}
                maxLength={SKILL_LIMITS.description}
                value={editing.skill.description}
                placeholder="Leave out personal data"
                onChange={(e) => update({ description: e.target.value })}
              />
            )}
          </Field>
          <Field
            label="Instruction"
            hint="Added to the AI request when you use this skill. Plain English, not SQL."
          >
            {(id) => (
              <textarea
                id={id}
                className={styles.textarea}
                rows={3}
                maxLength={SKILL_LIMITS.instruction}
                value={editing.skill.instruction}
                placeholder="Never select email, phone or address columns."
                onChange={(e) => update({ instruction: e.target.value })}
              />
            )}
          </Field>
          {error && <Alert tone="danger">{error}</Alert>}
          <div className={styles.actions}>
            <Button size="sm" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button size="sm" variant="primary" onClick={save}>
              Save skill
            </Button>
          </div>
        </div>
      ) : (
        <>
          {error && <Alert tone="danger">{error}</Alert>}
          <div>
            <Button size="sm" icon={Plus} onClick={() => edit(blank, null)}>
              Create skill
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
