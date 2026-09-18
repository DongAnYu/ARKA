export const MAX_DISPLAY_NAME_LENGTH = 80

export type UserProfile = {
  display_name: string
}

export function userProfileError(displayName: string): string {
  if ([...displayName.trim()].length > MAX_DISPLAY_NAME_LENGTH) {
    return `Your name must be ${MAX_DISPLAY_NAME_LENGTH} characters or fewer.`
  }

  return ''
}
