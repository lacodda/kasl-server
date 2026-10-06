import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { CheckCheck, Hourglass } from 'lucide-react'
import { api, ApiError, type ReportQueue, type WaitingReport } from '@/lib/api'
import { duration, weekdayName } from '@/lib/day'
import { Panel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'

/**
 * The days waiting for the reader's approval, on the dashboard (ADR 0022).
 *
 * Under the alerts and above the signals. An alert decays - a machine quiet
 * for thirty hours gets less actionable every hour - and goes first; a report
 * is a question somebody asked and is waiting on, which is more pressing than
 * a slide over three weeks and less than a fire.
 *
 * Each row approves on its own, and the heading approves everything shown in
 * one request: approving the week is what a manager does on a Friday. Sending
 * a day back needs words, so it is done on the day itself - the row opens it.
 *
 * Nothing at all where the installation does not approve days: a band saying
 * "nothing waiting" there would suggest something could.
 */
export function Approvals() {
  const { t } = useTranslation()
  const [queue, setQueue] = useState<ReportQueue | null>(null)
  const [failed, setFailed] = useState(false)
  const [answering, setAnswering] = useState<string[]>([])
  const [refused, setRefused] = useState<string[]>([])

  const load = useCallback(() => {
    let cancelled = false
    api
      .teamReports()
      .then((answer) => {
        if (!cancelled) setQueue(answer)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => load(), [load])

  const approve = useCallback(
    async (ids: string[]) => {
      setAnswering(ids)
      setRefused([])
      try {
        const outcome = await api.approveReports(ids)
        // The server's reasons, grouped: "the day changed after it was
        // reported" five times is one sentence with a count.
        setRefused([...new Set(outcome.refused.map((refusal) => refusal.error))])
      } catch (error) {
        setRefused([error instanceof ApiError ? error.message : t('common.error')])
      } finally {
        setAnswering([])
        // Asked again rather than edited locally: what waits now is the
        // server's answer, and a report refused because its day moved has
        // left the queue for a reason this screen cannot know.
        load()
      }
    },
    [load, t],
  )

  // A band that failed to load says nothing rather than "nothing waiting":
  // the reassuring reading of an error is the wrong one.
  if (failed || !queue || !queue.day_approval) return null

  if (queue.reports.length === 0) {
    return (
      <Panel className="flex items-center gap-2 px-5 py-3.5">
        <CheckCheck className="size-3.5 shrink-0 text-faint" aria-hidden />
        <p className="text-sm text-dim">{t('reports.queue.nothing')}</p>
      </Panel>
    )
  }

  const shown = queue.reports.map((report) => report.id)
  const busy = answering.length > 0

  return (
    <Panel className="divide-y divide-line">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2 px-5 pb-2 pt-3.5">
        <h2 className="flex items-baseline gap-2 text-xs font-medium text-dim">
          {t('reports.queue.title')}
          <span className="text-2xs text-faint tabular">{t('reports.queue.count', { count: queue.waiting })}</span>
        </h2>
        {shown.length > 1 && (
          <Button size="sm" variant="primary" disabled={busy} onClick={() => void approve(shown)}>
            {t('reports.queue.approveAll', { count: shown.length })}
          </Button>
        )}
      </div>
      {queue.reports.map((report) => (
        <WaitingRow
          key={report.id}
          report={report}
          busy={busy}
          answering={answering.includes(report.id)}
          onApprove={() => void approve([report.id])}
        />
      ))}
      {(queue.waiting > shown.length || refused.length > 0) && (
        <div className="space-y-1 px-5 py-2.5 text-xs">
          {queue.waiting > shown.length && <p className="text-faint">{t('reports.queue.more', { count: queue.waiting - shown.length })}</p>}
          {refused.length > 0 && (
            <p className="text-bad">{t('reports.queue.refused', { count: refused.length, reasons: refused.join('; ') })}</p>
          )}
        </div>
      )}
    </Panel>
  )
}

function WaitingRow({
  report,
  busy,
  answering,
  onApprove,
}: {
  report: WaitingReport
  busy: boolean
  answering: boolean
  onApprove: () => void
}) {
  const { t } = useTranslation()
  const figure = report.kind === 'work' ? duration(report.worked_seconds) : t(`myDay.dayKind.${report.kind}`)

  return (
    <div className="flex items-start gap-3 px-4 py-2.5 sm:items-center sm:px-5">
      <Hourglass className="mt-0.5 size-3.5 shrink-0 text-dim sm:mt-0" aria-hidden />
      {/* The day itself, opened, on the drill-down - where it can be read in
          full and sent back with a reason. The name and the day stack on a
          phone, as the alerts' rows do. */}
      <Link
        to={`/team/${report.user_id}?date=${report.date}`}
        className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-[9px] transition-colors hover:text-text sm:flex-row sm:items-center sm:gap-3"
        aria-label={t('reports.queue.rowLabel', { name: report.display_name, date: report.date, hours: figure })}
        title={t('reports.queue.look')}
      >
        <span className="min-w-0 truncate text-sm font-medium sm:w-44 sm:shrink-0">{report.display_name}</span>
        <span className="flex min-w-0 items-baseline gap-3 text-sm text-dim sm:flex-1">
          <span className="shrink-0">
            {weekdayName(report.date)} <span className="font-mono text-xs text-faint tabular">{report.date.slice(5)}</span>
          </span>
          <span className="font-mono text-sm text-text tabular">{figure}</span>
        </span>
      </Link>
      {report.department && <span className="hidden shrink-0 text-[11px] text-faint sm:block">{report.department}</span>}
      <Button
        variant="ghost"
        size="sm"
        className="shrink-0"
        disabled={busy}
        onClick={onApprove}
        // Five buttons reading "Approve" are five of the same to a screen
        // reader; each says whose day it answers.
        aria-label={t('reports.queue.approveRow', { name: report.display_name, date: report.date })}
      >
        {t(answering ? 'reports.approving' : 'reports.approve')}
      </Button>
    </div>
  )
}
