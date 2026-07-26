import { zodResolver } from '@hookform/resolvers/zod'
import { Check, ChevronsUpDown } from 'lucide-react'
import { useState } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod/v3'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Textarea } from '@/components/ui/textarea'
import {
  ONBOARDING_ABOUT_SUBMITTED_EVENT,
  ONBOARDING_STEP_COMPLETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { onboardingProfileStorage } from '@/lib/onboarding/onboardingStorage'
import { personalizationStorage } from '@/lib/personalization/personalizationStorage'
import { cn } from '@/lib/utils'
import { type StepDirection, StepTransition } from './StepTransition'

export interface StepOneProps {
  direction: StepDirection
  onContinue: () => void
}

const roles = [
  'Software Engineer',
  'Frontend Engineer',
  'Backend Engineer',
  'Full Stack Engineer',
  'DevOps Engineer',
  'Data Engineer',
  'ML Engineer',
  'Engineering Manager',
  'Tech Lead',
  'CTO',
  'VP of Engineering',
  'Product Manager',
  'Product Designer',
  'UX Researcher',
  'QA Engineer',
  'Solutions Architect',
  'Developer Advocate',
  'Data Scientist',
  'Founder / Co-Founder',
  'CEO',
  'COO',
  'Growth / Marketing',
  'Sales Engineer',
  'Customer Success',
]

const formSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  role: z.string().min(1, 'Role is required'),
  company: z.string().min(1, 'Company is required'),
  description: z.string().optional(),
})

type FormValues = z.infer<typeof formSchema>

export const StepOne = ({ direction, onContinue }: StepOneProps) => {
  const [roleOpen, setRoleOpen] = useState(false)
  const [roleSearch, setRoleSearch] = useState('')

  const form = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: {
      name: '',
      role: '',
      company: '',
      description: '',
    },
  })

  const handleSubmit = async (values: FormValues) => {
    const name = values.name.trim()
    const role = values.role.trim()
    const company = values.company.trim()
    const description = values.description?.trim() || undefined

    await onboardingProfileStorage.setValue({
      name,
      role,
      company,
      description,
    })

    const parts: string[] = []
    parts.push(`Name: ${name}`)
    parts.push(`Role: ${role}`)
    parts.push(`Company: ${company}`)
    if (description) parts.push(`About: ${description}`)
    await personalizationStorage.setValue(parts.join('\n'))

    track(ONBOARDING_ABOUT_SUBMITTED_EVENT, {
      fields_filled: parts.length,
      has_name: true,
      has_role: true,
      has_company: true,
      has_description: !!description,
      role,
    })

    track(ONBOARDING_STEP_COMPLETED_EVENT, { step: 1, step_name: 'about' })
    onContinue()
  }

  return (
    <StepTransition direction={direction}>
      <div className="flex h-full flex-col items-center justify-center">
        <div className="w-full max-w-md space-y-6">
          <div className="space-y-2 text-center">
            <h2 className="font-bold text-3xl tracking-tight">
              Tell us about yourself
            </h2>
            <p className="text-base text-muted-foreground">
              Help us personalize your experience
            </p>
          </div>

          <Form {...form}>
            <form
              onSubmit={form.handleSubmit(handleSubmit)}
              className="space-y-4"
            >
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Your name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="What should we call you?"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="role"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Your role</FormLabel>
                    <Popover open={roleOpen} onOpenChange={setRoleOpen}>
                      <PopoverTrigger asChild>
                        <FormControl>
                          <Button
                            variant="outline"
                            className="w-full justify-between font-normal"
                          >
                            {field.value || (
                              <span className="text-muted-foreground">
                                Select or type a role
                              </span>
                            )}
                            <ChevronsUpDown className="ml-2 size-4 shrink-0 opacity-50" />
                          </Button>
                        </FormControl>
                      </PopoverTrigger>
                      <PopoverContent
                        className="p-0"
                        style={{
                          width: 'var(--radix-popover-trigger-width)',
                        }}
                      >
                        <Command>
                          <CommandInput
                            placeholder="Search roles..."
                            value={roleSearch}
                            onValueChange={setRoleSearch}
                          />
                          <CommandList>
                            <CommandEmpty className="p-0" />
                            <CommandGroup>
                              {roleSearch.trim() &&
                                !roles.some(
                                  (r) =>
                                    r.toLowerCase() ===
                                    roleSearch.trim().toLowerCase(),
                                ) && (
                                  <CommandItem
                                    value={roleSearch.trim()}
                                    onSelect={() => {
                                      field.onChange(roleSearch.trim())
                                      setRoleOpen(false)
                                      setRoleSearch('')
                                    }}
                                  >
                                    <Check className="size-4 opacity-0" />
                                    {roleSearch.trim()}
                                  </CommandItem>
                                )}
                              {roles.map((r) => (
                                <CommandItem
                                  key={r}
                                  value={r}
                                  onSelect={(value) => {
                                    field.onChange(
                                      value === field.value ? '' : value,
                                    )
                                    setRoleOpen(false)
                                    setRoleSearch('')
                                  }}
                                >
                                  <Check
                                    className={cn(
                                      'size-4',
                                      field.value === r
                                        ? 'opacity-100'
                                        : 'opacity-0',
                                    )}
                                  />
                                  {r}
                                </CommandItem>
                              ))}
                            </CommandGroup>
                          </CommandList>
                        </Command>
                      </PopoverContent>
                    </Popover>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="company"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Company</FormLabel>
                    <FormControl>
                      <Input placeholder="Acme Inc." {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="description"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      What does a typical day look like for you?
                    </FormLabel>
                    <FormControl>
                      <Textarea
                        placeholder="I spend most of my day researching competitors, writing specs, and coordinating with engineering..."
                        rows={4}
                        className="field-sizing-fixed"
                        {...field}
                      />
                    </FormControl>
                  </FormItem>
                )}
              />

              <Button
                type="submit"
                className="w-full bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
              >
                Continue
              </Button>
            </form>
          </Form>
        </div>
      </div>
    </StepTransition>
  )
}
