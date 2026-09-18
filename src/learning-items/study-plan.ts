import type { LearningItem } from './types'

export type StudyPreferences = { daily_target: number; max_new_items: number }
export type PlannedItem = {
  was_new: boolean
  is_extra: boolean
  item: LearningItem
}
export type DailyStudyPlan = StudyPreferences & {
  local_date: string
  space_id: number | null
  completed_count: number
  total_count: number
  extra_completed_count: number
  can_study_more: boolean
  items: PlannedItem[]
}

export function studyPreferencesError(target: string, newItems: string): string {
  const daily = Number(target)
  const maximumNew = Number(newItems)
  if (!target.trim() || !Number.isInteger(daily) || daily < 1 || daily > 10000) {
    return 'Enter a daily study target between 1 and 10,000.'
  }
  if (!newItems.trim() || !Number.isInteger(maximumNew) || maximumNew < 0 || maximumNew > daily) {
    return 'Maximum new items must be between 0 and your daily study target.'
  }
  return ''
}
