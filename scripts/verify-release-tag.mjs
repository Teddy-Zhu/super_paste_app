import { verifyReleaseTag } from './project.mjs'

console.log(verifyReleaseTag(process.argv[2] ?? process.env.GITHUB_REF_NAME))
