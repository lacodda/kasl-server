import { Fragment, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Download } from 'lucide-react'
import { downloadExport, type ExportFile, type ExportSubject } from '@/lib/api'
import { isoDate } from '@/lib/day'
import { elapsed, type Period } from '@/lib/period'
import { Button } from '@/components/ui/button'
import { Menu, MenuItem, MenuPopup, MenuSeparator, MenuTrigger } from '@/components/ui/menu'

/** The files, in the order the menu offers them: the one people want first. */
const FILES: { file: ExportFile; label: string }[] = [
  { file: 'xlsx', label: 'export.workbook' },
  { file: 'summary.csv', label: 'export.summaryCsv' },
  { file: 'days.csv', label: 'export.daysCsv' },
]

/**
 * Downloads the period on screen as a file (ADR 0023).
 *
 * The period is the screen's: what is exported is what the reader is looking
 * at, measured the same way, so there is no second set of dates to pick and
 * get wrong. A menu rather
 * than three buttons - the workbook is what most people want, and the two CSVs
 * are for whoever feeds a program.
 */
export function ExportMenu({ subject, period }: { subject: ExportSubject; period: Period }) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)

  const download = (file: ExportFile) => {
    setBusy(true)
    setFailed(false)
    // What the screen shows: a period still running is exported to today, so
    // the file holds no dates that are due but have not come yet.
    const { from, to } = elapsed(period, isoDate(new Date())) ?? period
    downloadExport(subject, file, from, to)
      .catch(() => setFailed(true))
      .finally(() => setBusy(false))
  }

  return (
    <div className="flex items-center gap-2">
      {failed && (
        <span role="alert" className="text-xs text-bad">
          {t('export.failed')}
        </span>
      )}
      <Menu>
        <MenuTrigger
          render={
            // The phone's height for a finger, handed back at `sm`, as the
            // period's arrows beside it do.
            <Button size="sm" className="h-11 w-full sm:h-7 sm:w-auto" disabled={busy}>
              <Download aria-hidden className="size-3.5" />
              {busy ? t('export.preparing') : t('export.button')}
            </Button>
          }
        />
        <MenuPopup align="end">
          {FILES.map(({ file, label }, at) => (
            <Fragment key={file}>
              {/* The workbook holds both tables; the CSVs are one each. */}
              {at === 1 && <MenuSeparator />}
              <MenuItem onClick={() => download(file)}>{t(label)}</MenuItem>
            </Fragment>
          ))}
        </MenuPopup>
      </Menu>
    </div>
  )
}
