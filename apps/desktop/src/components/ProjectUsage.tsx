import { ChevronDown, FolderGit2, Layers3, LockKeyhole, Search } from 'lucide-react';
import { useId, useState } from 'react';
import type { ProjectTotal, ProviderID } from '../lib/types';
import { matchingProjects } from '../lib/projectUsage';
import { providerNames } from '../lib/preferences';
import { formatCost, formatTokens } from '../lib/usage';

export function ProviderProjects({ contentID, provider, projects, total, spend }: { contentID: string; provider: ProviderID; projects: ProjectTotal[]; total: number; spend: boolean }) {
  const [query, setQuery] = useState(''); const [visible, setVisible] = useState(6);
  const searchID = useId(); const assigned = projects.filter(project => project.directory);
  const coverage = total ? Math.floor(assigned.reduce((sum, project) => sum + project.tokens, 0) / total * 100) : 0;
  const matches = matchingProjects(projects, query);
  return <section id={contentID} className="provider-projects" aria-label={providerNames[provider] + ' Project Usage'} data-no-share="true">
    <div className="provider-projects-heading"><h3>Projects</h3><span>{assigned.length} {assigned.length === 1 ? 'Project' : 'Projects'}{total > 0 && ' · ' + coverage + '% Assigned'}</span></div>
    {projects.length > 0 ? <>
      {projects.length > 4 && <label className="project-search" htmlFor={searchID}><Search size={16} /><input id={searchID} type="search" aria-label={'Search ' + providerNames[provider] + ' Projects'} placeholder="Search projects…" value={query} onChange={event => { setQuery(event.target.value); setVisible(6); }} /></label>}
      <div className="project-list-heading"><span>Project / Models</span><span>Tokens{spend && ' / Cost'}</span><span>Of {providerNames[provider]}</span></div>
      <div className="project-list">{matches.slice(0, visible).map(project => <ProjectRow key={project.id} project={project} spend={spend} />)}</div>
      {!matches.length && <p className="project-empty" role="status">No projects match “{query}”.</p>}
      {matches.length > visible && <button className="text-button project-show-more" onClick={() => setVisible(count => count + 6)}>Show More ({matches.length - visible} Remaining)</button>}
      {projects.some(project => !project.directory) && <p className="project-unassigned-note">{provider === 'cursor' ? 'Cursor reports account-wide usage without project folders.' : 'Unassigned usage has no project folder in the source records.'} It is included in this provider’s total.</p>}
    </> : <p className="project-empty">No measured usage in this period.{provider === 'cursor' ? ' Cursor reports account-wide usage without project folders.' : ' Project folders appear when your tools record them.'}</p>}
    <p className="project-local-note"><LockKeyhole size={12} />Project folders stay on this device and are excluded from sharing.</p>
  </section>;
}

function ProjectRow({ project, spend }: { project: ProjectTotal; spend: boolean }) {
  const Icon = project.directory ? FolderGit2 : Layers3;
  return <details className="project-row">
    <summary aria-label={project.name + ' Usage Details'}><Icon className="project-folder-icon" size={17} />
      <span className="project-identity"><strong>{project.name}</strong><small>{project.models.length} {project.models.length === 1 ? 'Model' : 'Models'}</small></span>
      <span className="project-amount"><strong>{formatTokens(project.tokens)}</strong>{spend && <small>{formatCost(project.estimatedCost, project.unpricedTokens)}</small>}</span>
      <span className="project-share"><span className="meter"><i style={{ width: Math.min(100, project.share) + '%' }} /></span><small>{Math.round(project.share)}%</small></span>
      <ChevronDown size={15} className="project-row-chevron" />
    </summary>
    <div className="project-details">
      {project.directory ? <p className="project-directory"><span>Folder</span><code>{project.directory}</code></p> : <p className="project-unassigned-detail">These records do not include a project folder.</p>}
      <section className="project-models"><h4>Models</h4>{project.models.map(model => <div className="project-model-row" key={model.model}><span>{model.model}</span><span><strong>{formatTokens(model.tokens)}</strong>{spend && <small>{formatCost(model.estimatedCost, model.unpricedTokens)}</small>}</span></div>)}</section>
    </div>
  </details>;
}
