import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { api, type PrivacyManifest } from '@/lib/api'
import { Panel } from '@/components/ui/Panel'

/**
 * The privacy manifest, for the person it is about.
 *
 * The server generates it from the level it actually enforces (ADR 0011), so
 * this page renders what it is given rather than describing the levels itself:
 * a screen that explained the policy in its own words could disagree with the
 * server, and the employee would have no way to tell which was true.
 */
export function Privacy() {
  const { t } = useTranslation()
  const [manifest, setManifest] = useState<PrivacyManifest | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    api
      .privacy()
      .then((value) => {
        if (!cancelled) setManifest(value)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  if (failed) return <p className="text-sm text-bad">{t('common.error')}</p>
  if (!manifest) return <p className="text-sm text-dim">{t('common.loading')}</p>

  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <div>
        <h1 className="text-lg font-semibold">{t('privacy.title')}</h1>
        <p className="mt-2 text-sm text-dim">{manifest.summary}</p>
      </div>

      <Panel className="p-5">
        <div className="flex items-baseline justify-between gap-4">
          <span className="text-xs font-medium text-dim">{t('privacy.level')}</span>
          <span className="rounded-[9px] bg-accent-soft px-2.5 py-1 font-mono text-xs text-accent-2">
            {t(`privacy.levels.${manifest.level}`)}
          </span>
        </div>
      </Panel>

      <Section title={t('privacy.stored')}>
        <dl className="space-y-3">
          {manifest.stored.map((item) => (
            <div key={item.what}>
              <dt className="font-mono text-xs text-accent-2">{item.what}</dt>
              <dd className="mt-0.5 text-sm text-dim">{item.detail}</dd>
            </div>
          ))}
        </dl>
      </Section>

      <Section title={t('privacy.neverCollected')}>
        <ul className="space-y-1.5">
          {manifest.never_collected.map((item) => (
            <li key={item} className="text-sm text-dim">
              {item}
            </li>
          ))}
        </ul>
      </Section>

      <Section title={t('privacy.visibleTo')}>
        <ul className="space-y-1.5">
          {manifest.visible_to.map((item) => (
            <li key={item} className="text-sm text-dim">
              {item}
            </li>
          ))}
        </ul>
      </Section>

      <Section title={t('privacy.retention')}>
        <p className="text-sm text-dim">{manifest.retention}</p>
      </Section>

      <Section title={t('privacy.onChange')}>
        <p className="text-sm text-dim">{manifest.on_change}</p>
        {manifest.updated_at && (
          <p className="mt-3 font-mono text-xs text-faint tabular">
            {t('privacy.updatedAt')}: {new Date(manifest.updated_at).toLocaleString()}
          </p>
        )}
      </Section>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Panel className="p-5">
      <h2 className="text-xs font-medium tracking-wide text-dim uppercase">{title}</h2>
      <div className="mt-3">{children}</div>
    </Panel>
  )
}
