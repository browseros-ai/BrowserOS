import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import {
  AlertCircle,
  ArrowLeft,
  Camera,
  CheckCircle2,
  CircleUser,
  Loader2,
  UserPen,
} from 'lucide-react'
import type { FC } from 'react'
import { useEffect, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useNavigate } from 'react-router'
import { z } from 'zod/v3'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { env } from '@/lib/env'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { useGraphqlMutation } from '@/modules/graphql/graphql-mutation.hooks'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import {
  GetProfileByUserIdDocument,
  UpdateProfileByUserIdDocument,
} from './graphql/profileDocument'

const formSchema = z.object({
  firstName: z.string().min(1, 'First name is required'),
  lastName: z.string().min(1, 'Last name is required'),
})

type FormValues = z.infer<typeof formSchema>

type ProfileState = 'idle' | 'loading' | 'success' | 'error'

export const ProfilePage: FC = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { sessionInfo } = useSessionInfo()
  const fileInputRef = useRef<HTMLInputElement>(null)

  const userId = sessionInfo?.user?.id
  const isLoggedIn = !!userId

  const [avatarUrl, setAvatarUrl] = useState<string | null>(null)
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null)
  const [isUploading, setIsUploading] = useState(false)
  const [state, setState] = useState<ProfileState>('idle')
  const [error, setError] = useState<string | null>(null)

  const form = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: {
      firstName: '',
      lastName: '',
    },
  })

  const { data: profileData, isLoading: isLoadingProfile } = useGraphqlQuery(
    GetProfileByUserIdDocument,
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled
    { userId: userId! },
    { enabled: !!userId },
  )

  const updateProfileMutation = useGraphqlMutation(
    UpdateProfileByUserIdDocument,
    {
      onSuccess: () => {
        queryClient.invalidateQueries({
          queryKey: [getQueryKeyFromDocument(GetProfileByUserIdDocument)],
        })
        setState('success')
        setTimeout(() => setState('idle'), 3000)
      },
      onError: (err) => {
        setState('error')
        setError(
          err instanceof Error ? err.message : 'Failed to update profile',
        )
      },
    },
  )

  useEffect(() => {
    if (profileData?.profileByUserId) {
      const profile = profileData.profileByUserId
      form.reset({
        firstName: profile.firstName || '',
        lastName: profile.lastName || '',
      })
      setAvatarUrl(profile.avatarUrl || null)
    }
  }, [profileData, form])

  useEffect(() => {
    if (!isLoggedIn && !sessionInfo) {
      navigate('/login', { replace: true })
    }
  }, [isLoggedIn, sessionInfo, navigate])

  useEffect(() => {
    return () => {
      if (avatarPreview) URL.revokeObjectURL(avatarPreview)
    }
  }, [avatarPreview])

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return

    if (!file.type.startsWith('image/')) {
      setError('Please select an image file')
      return
    }

    if (file.size > 5 * 1024 * 1024) {
      setError('Image must be less than 5MB')
      return
    }

    setIsUploading(true)
    setError(null)

    const previewUrl = URL.createObjectURL(file)
    setAvatarPreview(previewUrl)

    try {
      const res = await fetch(
        `${env.VITE_PUBLIC_BROWSEROS_API}/upload/presigned-url`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'include',
          body: JSON.stringify({ contentType: file.type }),
        },
      )

      if (!res.ok) throw new Error('Failed to get upload URL')

      const { presignedUrl, publicUrl, headers } = await res.json()

      const uploadRes = await fetch(presignedUrl, {
        method: 'PUT',
        headers,
        body: file,
      })

      if (!uploadRes.ok) throw new Error('Failed to upload image')

      setAvatarUrl(publicUrl)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to upload image')
      setAvatarPreview(null)
    } finally {
      setIsUploading(false)
    }
  }

  const handleAvatarClick = () => {
    fileInputRef.current?.click()
  }

  const onSubmit = (values: FormValues) => {
    if (!userId) return

    setState('loading')
    setError(null)

    updateProfileMutation.mutate({
      userId,
      patch: {
        firstName: values.firstName.trim(),
        lastName: values.lastName.trim(),
        avatarUrl: avatarUrl,
      },
    })
  }

  const getInitials = () => {
    const first = form.watch('firstName').trim()[0] || ''
    const last = form.watch('lastName').trim()[0] || ''
    return (first + last).toUpperCase()
  }

  if (!isLoggedIn) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <Loader2 className="size-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (isLoadingProfile) {
    return (
      <Card className="w-full max-w-md">
        <CardContent className="flex min-h-[420px] items-center justify-center">
          <Loader2 className="size-8 animate-spin text-muted-foreground" />
        </CardContent>
      </Card>
    )
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => navigate(-1)}
            className="shrink-0"
          >
            <ArrowLeft className="size-4" />
          </Button>
          <div className="flex-1 pr-9 text-center">
            <CardTitle className="text-2xl">Update Profile</CardTitle>
            <CardDescription>Update your profile information</CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-6">
        {state === 'success' && (
          <Alert className="border-green-500 bg-green-50 dark:bg-green-950/30">
            <CheckCircle2 className="size-4 text-green-600 dark:text-green-400" />
            <AlertDescription className="text-green-600 dark:text-green-400">
              Profile updated successfully!
            </AlertDescription>
          </Alert>
        )}

        {error && (
          <Alert variant="destructive">
            <AlertCircle className="size-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-6">
            <div className="flex flex-col items-center gap-2">
              <button
                type="button"
                onClick={handleAvatarClick}
                disabled={isUploading || state === 'loading'}
                className="group relative cursor-pointer"
              >
                {avatarPreview || avatarUrl ? (
                  <img
                    // biome-ignore lint/style/noNonNullAssertion: guarded by ternary
                    src={avatarPreview || avatarUrl!}
                    alt="Profile"
                    className="size-24 rounded-full object-cover transition-opacity group-hover:opacity-80"
                  />
                ) : getInitials() ? (
                  <div className="flex size-24 items-center justify-center rounded-full bg-primary font-semibold text-2xl text-primary-foreground transition-opacity group-hover:opacity-80">
                    {getInitials()}
                  </div>
                ) : (
                  <div className="flex size-24 items-center justify-center rounded-full bg-muted transition-opacity group-hover:opacity-80">
                    <CircleUser className="size-12 text-muted-foreground" />
                  </div>
                )}
                <div className="absolute right-0 bottom-0 flex size-8 items-center justify-center rounded-full border-2 border-background bg-primary text-primary-foreground transition-transform group-hover:scale-110">
                  {isUploading ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Camera className="size-4" />
                  )}
                </div>
              </button>
              <p className="text-muted-foreground text-xs">
                Click to upload profile picture
              </p>
              <input
                ref={fileInputRef}
                type="file"
                accept="image/*"
                onChange={handleFileChange}
                className="hidden"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <FormField
                control={form.control}
                name="firstName"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>First Name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder=""
                        disabled={state === 'loading'}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="lastName"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Last Name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder=""
                        disabled={state === 'loading'}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>

            <Button
              type="submit"
              className="w-full"
              disabled={state === 'loading' || isUploading}
            >
              {state === 'loading' ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <UserPen className="size-4" />
              )}
              Save Changes
            </Button>
          </form>
        </Form>
      </CardContent>
    </Card>
  )
}
