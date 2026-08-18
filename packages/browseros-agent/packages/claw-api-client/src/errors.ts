export class ApiResponseError extends Error {
  override name = 'ApiResponseError'

  constructor(public readonly response: Response) {
    super(
      `BrowserOS neo API request failed with status ${response.status.toString()}`,
    )
  }
}
