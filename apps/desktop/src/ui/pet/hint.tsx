import { useTranslation } from 'react-i18next'
import { petI18n } from '@/pet/i18n'

export interface HintProps {
  petId: string
}

export function Hint(props: HintProps) {
  const { t } = useTranslation(undefined, { i18n: petI18n })

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-6 flex justify-center select-none">
      <div className="max-w-[95%] rounded-lg bg-black/70 px-3 py-1.5 text-center text-xs leading-relaxed text-white">
        {t('pet.unavailable', { id: props.petId })}
      </div>
    </div>
  )
}
