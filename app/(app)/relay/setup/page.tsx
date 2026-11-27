'use client'

import { IconArticle } from '@douyinfe/semi-icons'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import styles from '@/app/ui/relay/relay.module.scss'
import Wizard from './Wizard'

export default function RelaySetupPage() {
  return (
    <div className={styles.content}>
      <PageHeader
        title="首次设置向导"
        description="选择 B 站账号、填写基础参数并建立初始频道与分区"
        icon={<IconArticle />}
        breadcrumbs={[
          { href: '/relay', label: '直播转播' },
          { href: '/relay/setup', label: '首次设置' },
        ]}
      />

      <PermissionGate perm="relay.config">
        <Wizard />
      </PermissionGate>
    </div>
  )
}
