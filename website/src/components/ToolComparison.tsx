import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';
import React from 'react';

type Props = {
  pleaseCode: string;
  makeCode: string;
  justCode: string;
  language?: string;
};

export default function ToolComparison({
  pleaseCode,
  makeCode,
  justCode,
  language = 'bash',
}: Props): JSX.Element {
  return (
    <Tabs groupId="tool-compare" queryString>
      <TabItem value="please" label="Please">
        <pre>
          <code className={`language-${language}`}>{pleaseCode}</code>
        </pre>
      </TabItem>
      <TabItem value="make" label="Make">
        <pre>
          <code className={`language-${language}`}>{makeCode}</code>
        </pre>
      </TabItem>
      <TabItem value="just" label="Just">
        <pre>
          <code className={`language-${language}`}>{justCode}</code>
        </pre>
      </TabItem>
    </Tabs>
  );
}
